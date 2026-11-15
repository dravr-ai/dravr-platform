// ABOUTME: push_training_plan tool — puts the athlete's active plan on their provider calendar and reconciles the ledger
// ABOUTME: Also the same push a save or a coach's publish ends with, the calendar block get_training_plan shows and the save's preview
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::annotations::{annotations, Effect, Reach};
use pierre_providers::ai_scope;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::NaiveDate;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::TenantId;
use pierre_memory::training_plans::{parse_plan_date, PlanWeek};
use pierre_services::athlete_clock::athlete_today;
use pierre_services::coach_plan_draft::{draft_state, CoachDraftTarget};
use pierre_services::coach_plan_notice::{tell_plan_pushed, CoachCalendarPush};
use pierre_services::plan_calendar_push::{
    calendar_holds_week_notes, desired_entries, diff_against_ledger, for_calendar,
    push_active_plan, PushPlanParams, PushReport,
};
use pierre_services::plan_fueling::FuelingDisclosure;
use serde_json::{json, Value};
use tracing::warn;

use super::calendar::{
    calendar_ledger_target, provider_arg, provider_property, CalendarConnections, LedgerCalendar,
    PassedOver,
};
use super::coach_scope::{athlete_property, resolve_calendar_scope, under_athlete_consents};
use super::plan_scope::PlanScope;
use super::training_plan_telemetry::{emit_calendar_sync_completed, emit_calendar_sync_failed};
use super::training_plans_output::{
    CalendarBlock, CalendarEntry, CalendarPreview, CalendarPushOutcome, CalendarPusher,
    NO_CALENDAR, PUSH_FAILED,
};
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, object_schema, ok_typed, task_capable, tool_definition, tool_result_to_response,
};
use crate::guardian::{caller_tool_enabled, tenant_disabled_message, TENANT_DISABLED_REASON};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use crate::task_cancellation::current_task_cancel_flag;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_mcp_schema::PropertySchema;
use pierre_tools_core::ToolResult;

/// Tool name, as the notify events name the trigger.
const PUSH_TOOL: &str = "push_training_plan";

/// The calendar block: every entry Dravr has live on the athlete's calendar
/// from `today` on, and what a push would change.
///
/// Each entry carries its `prescription_id` so a later turn can name it to
/// `prescribe_workout`'s `replaces` or to `withdraw_prescribed_workout`, and
/// its `source` so the model knows which entries belong to the plan instead.
/// The days render with the push's own disclosure
/// ([`FuelingDisclosure::for_athletes_calendar`]), so the pending counts match
/// what the push would write whoever reads them. `scope` is whose plan it is
/// and who reads it: an athlete reading the calendar their human coach's link
/// holds is told the coach pushes it ([`CalendarBlock::pushed_by`]).
///
/// # Errors
///
/// Returns an error when the ledger cannot be read.
pub(super) async fn calendar_block(
    context: &ToolExecutionContext,
    scope: &PlanScope,
    active_weeks: &[PlanWeek],
    today: NaiveDate,
) -> AppResult<CalendarBlock> {
    let repos = context.resources.repos();
    let (tenant, user_id) = (scope.tenant, scope.user_id);
    let fueling = FuelingDisclosure::for_athletes_calendar(repos, tenant, user_id).await?;
    let ledger =
        calendar_ledger_target(context, tenant, user_id, ledger_connections(scope)).await?;
    let pushed_by = who_pushes(scope, &ledger);
    let calendar_note = read_note(context, scope, &ledger).await?;
    let target = ledger.backend;
    let mut live = repos
        .prescribed_workouts
        .list_live_calendar_events(tenant, user_id, &target, Some(today))
        .await?;
    // An entry pushed from first-party-only data is withheld from an external
    // caller; the rest are served (carnet#769).
    ai_scope::retain_admitted(&mut live, |row| row.transport_policy);
    let desired = for_calendar(
        desired_entries(user_id, active_weeks, today, &fueling),
        calendar_holds_week_notes(context.resources.provider_registry(), &target),
    );
    let pending = diff_against_ledger(&desired, &live)?;
    let entries: Vec<CalendarEntry> = live
        .iter()
        .map(|row| {
            // The pushed payload names the entry whatever its source: a plan
            // day's session and a prescription's template both carry `name`.
            let name = serde_json::from_str::<Value>(&row.payload_json)
                .ok()
                .and_then(|payload| {
                    payload
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .or_else(|| row.template_slug.clone());
            CalendarEntry {
                prescription_id: row.id,
                date: row.prescribed_for_date.format("%Y-%m-%d").to_string(),
                source: row.source,
                name,
                provider_event_id: row.provider_event_id.clone(),
                pushed_at: row.updated_at,
            }
        })
        .collect();
    let mut block = CalendarBlock {
        provider: target,
        entries,
        pending,
        stale: pending.is_stale(),
        scope: None,
        pushed_by,
        calendar_note,
    };
    if block.entries.is_empty() {
        // An empty ledger is not an empty calendar, and the difference is the
        // whole answer to "do I have a race today". This block is Dravr's record
        // of what Dravr pushed; nothing here reads the athlete's own calendar,
        // and for an athlete with no calendar provider connected it is empty
        // permanently. Said plainly next to the emptiness, because a model shown
        // `entries: []` under a key called `calendar` will otherwise report it as
        // "nothing on your calendar" — which it did, to an athlete asking whether
        // he had a race (2026-08-28).
        block.scope = Some(
            concat!(
                "Dravr has scheduled nothing. This lists only what Dravr pushed to ",
                "the athlete's calendar provider — it is NOT a view of their calendar. ",
                "An empty list says nothing about races or events they entered ",
                "themselves; say so rather than reporting they have none."
            )
            .to_owned(),
        );
    }
    Ok(block)
}

/// What a push would change, for the `save_training_plan` reply — only once
/// the calendar already carries this plan, so an athlete who never pushed is
/// not nagged about a calendar they do not use.
///
/// Best-effort like the ramp check: the plan is already committed, so an
/// unreadable ledger degrades to `None` rather than failing the save.
/// The days render with the push's own disclosure, and `scope` says who
/// pushes it, as [`calendar_block`] does.
pub(super) async fn calendar_preview_after_save(
    context: &ToolExecutionContext,
    scope: &PlanScope,
    plan_id: &str,
    today: NaiveDate,
) -> Option<CalendarPreview> {
    let repos = context.resources.repos();
    let (tenant, user_id) = (scope.tenant, scope.user_id);
    let ledger = best_effort(
        calendar_ledger_target(context, tenant, user_id, ledger_connections(scope)).await,
        "calendar target unresolved",
    )?;
    let pushed_by = who_pushes(scope, &ledger);
    let calendar_note = best_effort(
        read_note(context, scope, &ledger).await,
        "calendar note unresolved",
    )?;
    let target = ledger.backend;
    let mut live = best_effort(
        repos
            .prescribed_workouts
            .list_live_calendar_events(tenant, user_id, &target, Some(today))
            .await,
        "calendar ledger unreadable",
    )?;
    ai_scope::retain_admitted(&mut live, |row| row.transport_policy);
    if !live.iter().any(|row| row.source.is_plan()) {
        return None;
    }
    let weeks = best_effort(
        repos
            .training_plans
            .list_plan_weeks(&tenant.to_string(), &user_id.to_string(), plan_id, false)
            .await,
        "plan weeks unreadable for the calendar preview",
    )?;
    let fueling = best_effort(
        FuelingDisclosure::for_athletes_calendar(repos, tenant, user_id).await,
        "medical flag unreadable for the calendar preview",
    )?;
    let desired = for_calendar(
        desired_entries(user_id, &weeks, today, &fueling),
        calendar_holds_week_notes(context.resources.provider_registry(), &target),
    );
    let pending = best_effort(
        diff_against_ledger(&desired, &live),
        "calendar preview could not be computed",
    )?;
    Some(CalendarPreview {
        provider: target,
        pending,
        stale: pending.is_stale(),
        pushed_by,
        calendar_note,
    })
}

/// The calendar half of the athlete's own save reply (carnet#1492): the save
/// put on their chosen calendar as their own push would put it, or why it
/// was not — and, when nothing was pushed, what a push would now change
/// ([`calendar_preview_after_save`]).
///
/// Two refusals say nothing at all, since they would repeat on every save: an
/// athlete with no calendar Dravr writes is not nagged about one, and one
/// whose tenant switched `push_training_plan` off is not told so each time
/// (Phil, 2026-11-12). A coach's publish still says both.
pub(super) async fn calendar_after_own_save(
    context: &ToolExecutionContext,
    scope: &PlanScope,
    plan_id: &str,
    today: NaiveDate,
) -> (Option<CalendarPushOutcome>, Option<CalendarPreview>) {
    let pushed = Box::pin(push_after_write(
        context,
        &json!({}),
        "save_training_plan",
        AthleteNotice::Tell,
    ))
    .await;
    let unsaid = [NO_CALENDAR, TENANT_DISABLED_REASON]
        .iter()
        .any(|code| pushed.refused_for(code));
    let calendar_push = (!unsaid).then_some(pushed);
    let preview = match &calendar_push {
        Some(CalendarPushOutcome::Pushed { .. }) => None,
        Some(CalendarPushOutcome::NotPushed { .. }) | None => {
            calendar_preview_after_save(context, scope, plan_id, today).await
        }
    };
    (calendar_push, preview)
}

/// The note a read of `ledger` carries when it passed the athlete's chosen
/// calendar over; `None` when it did not.
///
/// # Errors
///
/// Returns the repository error when the connections cannot be read.
async fn read_note(
    context: &ToolExecutionContext,
    scope: &PlanScope,
    ledger: &LedgerCalendar,
) -> AppResult<Option<String>> {
    let Some(chosen) = ledger.passed_over.as_deref() else {
        return Ok(None);
    };
    let chooser = scope
        .acting_for()
        .map_or_else(|| "the athlete".to_owned(), str::to_owned);
    let passed = PassedOver::read(context, scope.tenant, scope.user_id, chooser, chosen).await?;
    Ok(Some(passed.read_note(context, &ledger.backend)))
}

/// The connections a read of `scope`'s ledger looks under: the calendars a
/// coach writes when a group's human coach reads for the athlete, the
/// athlete's own and their link's otherwise.
fn ledger_connections(scope: &PlanScope) -> CalendarConnections {
    if scope.acting_for().is_some() {
        CalendarConnections::ForCoach
    } else {
        CalendarConnections::WithLink
    }
}

/// Who pushes `ledger`'s calendar, when it is not the reader: an athlete
/// reading, in their own scope, the calendar they hold only through their
/// human coach's link has it pushed by that coach. A coach reading for the
/// athlete pushes it themselves.
fn who_pushes(scope: &PlanScope, ledger: &LedgerCalendar) -> Option<CalendarPusher> {
    (scope.acting_for().is_none() && ledger.through_link).then_some(CalendarPusher::HumanCoach)
}

/// The preview is best-effort: a step that fails is logged with what it was
/// doing and the preview is dropped, never the save that preceded it.
fn best_effort<T>(result: AppResult<T>, what: &str) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(e) => {
            warn!(error = %e, "save_training_plan: {what}");
            None
        }
    }
}

/// `push_training_plan` — the athlete's active plan onto their calendar.
#[derive(Debug)]
pub struct PushTrainingPlanTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for PushTrainingPlanTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "from_date".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "First date to push, YYYY-MM-DD. Defaults to today in the athlete's \
                     calendar and is never earlier."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert("provider".to_owned(), provider_property());
        properties.insert("athlete".to_owned(), athlete_property());
        let schema = object_schema(properties, None);
        answers_with::<PushReport>(task_capable(tool_definition(
            "push_training_plan",
            "Put the athlete's active training plan on their calendar provider (Intervals.icu, \
             Wahoo or Garmin): creates missing days, updates changed ones, removes dropped ones, \
             leaves alone any the athlete edited there (Garmin reports no edits). Never touches \
             dates before today. save_training_plan and publish_training_plan already push \
             (their calendar_push says what landed): call this only when the athlete or coach \
             asks, e.g. once what stopped that push is fixed. Needs a saved plan and a connected \
             calendar-writing provider. A group's human \
             coach pushes a consenting athlete's plan with `athlete`, through their \
             Intervals.icu link: they must allow calendar writes on it, and Intervals.icu must \
             list the coach as their coach. Only the published plan is pushed, never a coach's \
             unpublished draft (the reply's unpublished_draft flags one).",
            schema,
            annotations(
                "Push training plan to calendar",
                Effect::Destructive,
                Reach::AthleteAccount,
            ),
        )))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::REQUIRES_PROVIDER
            | ToolCapabilities::WRITES_DATA
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result = push_active_plan_for(&context, &args, PUSH_TOOL, AthleteNotice::Tell)
            .await
            .and_then(|push| match push {
                PlanPush::Pushed(report) => ok_typed("push_training_plan", &report),
                PlanPush::Refused(refused) => Ok(refused),
            });
        tool_result_to_response(result)
    }
}

/// Whether a coach's push of an athlete's calendar sends the athlete a
/// notice of it. The caller's push of their own calendar sends none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AthleteNotice {
    /// The athlete is told their coach put workouts on their calendar.
    Tell,
    /// The push ends a publish whose own notice told the athlete: one
    /// publish, one notice.
    ToldByPublish,
}

/// What putting an athlete's active plan on their calendar came to.
pub(super) enum PlanPush {
    /// The push ran: what it changed.
    Pushed(PushReport),
    /// A gate refused it before anything on the athlete's calendar was read
    /// or written: the refusal, in the tool's error shape.
    Refused(ToolResult),
}

/// Put the active plan of the athlete `args` names — the caller's own when
/// it names none — on their calendar, for `trigger`.
///
/// The one body behind `push_training_plan`, a coach's publish and an
/// athlete's own save (carnet#1492). `args` is read as the tool takes it:
/// `athlete`, `provider` and `from_date`. The calendar scope's gates
/// ([`resolve_calendar_scope`]) run on every call under the tool's name, so a
/// coach writes an athlete's calendar only on Dravr's own surfaces and
/// outside a room whatever triggered the write, and the athlete's tenant is
/// held to its own `push_training_plan` setting. The athlete is told when
/// their coach wrote their calendar, as `notice` says; the notify events
/// name `trigger`.
///
/// # Errors
///
/// Returns an error when the athlete holds no calendar Dravr writes
/// (`NoProviderConnected`), `from_date` is not a canonical date, a lookup
/// fails, or the push itself fails.
pub(super) async fn push_active_plan_for(
    context: &ToolExecutionContext,
    args: &Value,
    trigger: &str,
    notice: AthleteNotice,
) -> AppResult<PlanPush> {
    // Whose calendar: the caller's own, or a coached athlete's; the ledger,
    // the plan and "today" are the athlete's.
    let mut scope = match resolve_calendar_scope(context, args, None, PUSH_TOOL).await? {
        Ok(scope) => scope,
        Err(refused) => return Ok(PlanPush::Refused(refused)),
    };
    // What the call reads of a coached athlete's calendar runs under the
    // athlete's AI consents, never the coach's (carnet#1387).
    let athlete_consents = scope.athlete_consents(context.resources.repos()).await;
    under_athlete_consents(athlete_consents, async {
        let (tenant, user_id) = (scope.tenant, scope.user_id);
        let repos = context.resources.repos();
        let today = athlete_today(repos, user_id).await;
        let from = match args.get("from_date").and_then(Value::as_str) {
            Some(raw) => parse_plan_date(raw)
                .ok_or_else(|| {
                    AppError::invalid_input(format!(
                        "from_date must be a canonical YYYY-MM-DD, got '{raw}'"
                    ))
                })?
                .max(today),
            None => today,
        };
        let chosen = match scope.target(context, provider_arg(args)).await? {
            Ok(chosen) => chosen,
            Err(ask) => return Ok(PlanPush::Refused(ask)),
        };
        let calendar_note = scope.passed_over_note(context, &chosen).await?;
        let target = chosen.backend;
        let provider = match scope.calendar_writer(context, &target).await? {
            Ok(provider) => provider,
            Err(refused) => return Ok(PlanPush::Refused(refused)),
        };

        // When this call runs behind an MCP task handle the dispatcher
        // scoped a cancel flag around it; handing it to the push loop is what
        // lets tasks/cancel stop the calendar writes between entries. An
        // inline call has no flag and pushes to completion.
        let cancel_flag = current_task_cancel_flag();
        let pushed = push_active_plan(
            repos,
            provider.as_ref(),
            &PushPlanParams {
                tenant,
                user_id,
                provider: &target,
                from,
                week_notes: calendar_holds_week_notes(
                    context.resources.provider_registry(),
                    &target,
                ),
                cancel: cancel_flag.as_deref(),
            },
        )
        .await;
        if let Err(error) = &pushed {
            emit_calendar_sync_failed(tenant, user_id, &target, trigger, &error.tool_message());
        }
        let mut report = scope.reacting(context, provider.as_ref(), pushed).await?;
        report.calendar_note = calendar_note;
        if let Some(name) = scope.athlete_name() {
            // The athlete did not push this: they are told their coach put
            // workouts on their calendar, when the push wrote any.
            if notice == AthleteNotice::Tell {
                #[cfg(feature = "client-notifications")]
                let notifications = context.resources.notification_service();
                #[cfg(not(feature = "client-notifications"))]
                let notifications = None;
                tell_plan_pushed(
                    repos,
                    context.resources.provider_registry(),
                    notifications,
                    &CoachCalendarPush {
                        tenant,
                        athlete: user_id,
                        coach: scope.writer.user_id,
                        backend: &target,
                    },
                    &report,
                )
                .await;
            }
            // A coach's draft is not the athlete's plan: the push carried the
            // published plan, and the coach is told the draft is still theirs.
            let target = CoachDraftTarget::new(tenant, user_id, scope.writer.user_id);
            if draft_state(repos, &target).await?.is_some() {
                report.unpublished_draft = Some(format!(
                    "You hold an unpublished draft of {name}'s plan — only the published plan \
                     is pushed. Publish the draft with publish_training_plan, then push again \
                     to put it on the calendar."
                ));
            }
        }

        let landed = report.created + report.updated + report.removed;
        let attempted = landed + report.failed.len();
        if let Some(first) = report.failed.first() {
            emit_calendar_sync_failed(
                tenant,
                user_id,
                &target,
                trigger,
                &format!(
                    "{} of {attempted} entries failed; first: {}",
                    report.failed.len(),
                    first.error
                ),
            );
        }
        if report.failed.is_empty() || landed > 0 {
            emit_calendar_sync_completed(tenant, user_id, &target, trigger, landed);
        }
        scope.record_coach_write(trigger, attempted);
        Ok(PlanPush::Pushed(report))
    })
    .await
}

/// The `reason` reported for a refusal by the plan scope's coaching gates,
/// which name none of their own: the athlete is not one the requester may
/// act for from here.
const COACHING_GATE: &str = "coaching_gate";

/// Put the athlete's active plan on their chosen calendar once a save or a
/// publish wrote it, for `trigger`: the outcome to report beside the write,
/// never a failure of it (carnet#1492).
///
/// The push runs through [`push_active_plan_for`], with every gate a call of
/// `push_training_plan` meets, and first the one the dispatch chokepoint
/// would have applied to that call: the requester's own setting for the
/// tool. A refusal is reported by its gate's reason, an athlete with no
/// calendar Dravr writes as [`NO_CALENDAR`], and a push that failed as
/// [`PUSH_FAILED`], each in the words a tool reply would carry. A coach's
/// push tells the athlete as `notice` says.
pub(super) async fn push_after_write(
    context: &ToolExecutionContext,
    args: &Value,
    trigger: &str,
    notice: AthleteNotice,
) -> CalendarPushOutcome {
    if let Ok(tenant) = context.require_tenant() {
        let tenant = TenantId::from_uuid(tenant);
        if !caller_tool_enabled(&context.resources, tenant, context.user_id, PUSH_TOOL).await {
            return CalendarPushOutcome::NotPushed {
                reason: TENANT_DISABLED_REASON.to_owned(),
                message: tenant_disabled_message(PUSH_TOOL),
            };
        }
    }
    match push_active_plan_for(context, args, trigger, notice).await {
        Ok(PlanPush::Pushed(report)) => CalendarPushOutcome::Pushed { report },
        Ok(PlanPush::Refused(refused)) => refused_outcome(&refused),
        Err(error) if error.code == ErrorCode::NoProviderConnected => {
            CalendarPushOutcome::NotPushed {
                reason: NO_CALENDAR.to_owned(),
                message: error.tool_message(),
            }
        }
        Err(error) => {
            warn!(
                trigger,
                error = %error.internal_details(),
                "the calendar push after a plan write failed; the write stands"
            );
            CalendarPushOutcome::NotPushed {
                reason: PUSH_FAILED.to_owned(),
                message: error.tool_message(),
            }
        }
    }
}

/// A gate's in-band refusal as the outcome a write reports: the gate's
/// reason, or [`COACHING_GATE`] for the plan scope's, and its words.
fn refused_outcome(refused: &ToolResult) -> CalendarPushOutcome {
    let field = |key: &str| refused.content.get(key).and_then(Value::as_str);
    CalendarPushOutcome::NotPushed {
        reason: field("reason").unwrap_or(COACHING_GATE).to_owned(),
        message: field("error").map_or_else(|| refused.content.to_string(), str::to_owned),
    }
}

/// Factory for the plan-push tool set.
#[must_use]
pub fn create_training_plan_push_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![Box::new(PushTrainingPlanTool)]
}

// A push deletes calendar entries the plan no longer wants, and a delete on
// the provider cannot be undone from here — re-pushing recreates the entry,
// but as a new one. Destructive, so it spends the Guardian's per-turn destructive budget.
crate::declare_security!(PushTrainingPlanTool => IRREVERSIBLE);
