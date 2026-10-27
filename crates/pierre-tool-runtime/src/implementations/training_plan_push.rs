// ABOUTME: push_training_plan tool — puts the athlete's active plan on their provider calendar and reconciles the ledger
// ABOUTME: Also renders the calendar block get_training_plan shows and the pending-change preview save_training_plan reports
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::annotations::{annotations, Effect, Reach};
use pierre_providers::ai_scope;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::NaiveDate;
use pierre_core::errors::{AppError, AppResult};
use pierre_memory::training_plans::{parse_plan_date, PlanWeek};
use pierre_services::athlete_clock::athlete_today;
use pierre_services::coach_plan_draft::{draft_state, CoachDraftTarget};
use pierre_services::coach_plan_notice::{tell_plan_pushed, CoachCalendarPush};
use pierre_services::plan_calendar_push::{
    calendar_holds_week_notes, desired_entries, diff_against_ledger, for_calendar,
    push_active_plan, PushPlanParams, PushReport,
};
use pierre_services::plan_fueling::FuelingDisclosure;
use serde_json::Value;
use tracing::warn;

use super::calendar::{calendar_ledger_target, provider_arg, provider_property, LedgerCalendar};
use super::coach_scope::{athlete_property, resolve_calendar_scope};
use super::plan_scope::PlanScope;
use super::training_plan_telemetry::{emit_calendar_sync_completed, emit_calendar_sync_failed};
use super::training_plans_output::{CalendarBlock, CalendarEntry, CalendarPreview, CalendarPusher};
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, object_schema, ok_typed, task_capable, tool_definition, tool_result_to_response,
};
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
/// `fueling` is the disclosure a push would render the days with, so the
/// pending counts match what the push would write. `scope` is whose plan it
/// is and who reads it: an athlete reading the calendar their human coach's
/// link holds is told the coach pushes it ([`CalendarBlock::pushed_by`]).
///
/// # Errors
///
/// Returns an error when the ledger cannot be read.
pub(super) async fn calendar_block(
    context: &ToolExecutionContext,
    scope: &PlanScope,
    active_weeks: &[PlanWeek],
    today: NaiveDate,
    fueling: &FuelingDisclosure,
) -> AppResult<CalendarBlock> {
    let repos = context.resources.repos();
    let (tenant, user_id) = (scope.tenant, scope.user_id);
    let ledger = calendar_ledger_target(context, tenant, user_id).await?;
    let pushed_by = who_pushes(scope, &ledger);
    let target = ledger.backend;
    let mut live = repos
        .prescribed_workouts
        .list_live_calendar_events(tenant, user_id, &target, Some(today))
        .await?;
    // An entry pushed from first-party-only data is withheld from an external
    // caller; the rest are served (carnet#769).
    ai_scope::retain_admitted(&mut live, |row| row.transport_policy);
    let desired = for_calendar(
        desired_entries(user_id, active_weeks, today, fueling),
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
/// `fueling` is the disclosure a push would render the days with; `scope`
/// says who pushes it, as [`calendar_block`] does.
pub(super) async fn calendar_preview_after_save(
    context: &ToolExecutionContext,
    scope: &PlanScope,
    plan_id: &str,
    today: NaiveDate,
    fueling: &FuelingDisclosure,
) -> Option<CalendarPreview> {
    let repos = context.resources.repos();
    let (tenant, user_id) = (scope.tenant, scope.user_id);
    let ledger = best_effort(
        calendar_ledger_target(context, tenant, user_id).await,
        "calendar target unresolved",
    )?;
    let pushed_by = who_pushes(scope, &ledger);
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
    let desired = for_calendar(
        desired_entries(user_id, &weeks, today, fueling),
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
    })
}

/// Who pushes `ledger`'s calendar, when it is not the reader: an athlete
/// reading, in their own scope, the calendar they hold only through their
/// human coach's link has it pushed by that coach. A coach reading for the
/// athlete pushes it themselves.
fn who_pushes(scope: &PlanScope, ledger: &LedgerCalendar) -> Option<CalendarPusher> {
    (scope.acting_for.is_none() && ledger.through_link).then_some(CalendarPusher::HumanCoach)
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
                    "First date to push (YYYY-MM-DD). Defaults to today in the athlete's \
                     calendar, and is never earlier than that: dates already past are not \
                     rewritten."
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
            "Put the athlete's active training plan on their calendar provider (Intervals.icu \
             today), or bring the calendar up to date after the plan changed: creates the \
             days that are missing, updates the ones that changed, removes the ones the \
             plan no longer has, and leaves alone any the athlete edited on the provider. Never \
             touches dates before today. Call it when the athlete or coach asks to put or \
             update the plan on their calendar — not on your own initiative after a save; \
             save_training_plan's reply says when the calendar is behind. Requires a saved \
             plan and a connected account of a provider that writes a training calendar. A \
             group's human coach pushes a consenting athlete's plan by passing `athlete` from \
             their own direct chat in Dravr, through the athlete's Intervals.icu link: the \
             athlete must have allowed calendar writes on it, and Intervals.icu must list the \
             coach as their coach. Only the published plan is pushed: a coach's unpublished \
             draft never is, and the reply's unpublished_draft says when one exists. Args: \
             optional from_date, optional provider (needed only to choose between several \
             connected calendars), optional athlete.",
            schema,
            annotations(Effect::Destructive, Reach::OpenWorld),
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
        let result: AppResult<ToolResult> = async move {
            // Whose calendar: the caller's own, or a coached athlete's; the
            // ledger, the plan and "today" are the athlete's.
            let mut scope = match resolve_calendar_scope(&context, &args, None, PUSH_TOOL).await? {
                Ok(scope) => scope,
                Err(refused) => return Ok(refused),
            };
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
            let target = scope.target(&context, provider_arg(&args)).await?;
            let provider = match scope.calendar_writer(&context, &target).await? {
                Ok(provider) => provider,
                Err(refused) => return Ok(refused),
            };

            // When this call runs behind an MCP task handle the dispatcher
            // scoped a cancel flag around it; handing it to the push loop is
            // what lets tasks/cancel stop the calendar writes between
            // entries. An inline call has no flag and pushes to completion.
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
            let mut report = scope.reacting(&context, provider.as_ref(), pushed).await?;
            if let Some(name) = scope.athlete_name() {
                // The athlete did not push this: they are told their coach
                // put workouts on their calendar, when the push wrote any.
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
                // A coach's draft is not the athlete's plan: the push carried
                // the published plan, and the coach is told the draft is
                // still theirs.
                let target = CoachDraftTarget::new(tenant, user_id, scope.writer.user_id);
                if draft_state(repos, &target).await?.is_some() {
                    report.unpublished_draft = Some(format!(
                        "You hold an unpublished draft of {name}'s plan — only the published \
                         plan is pushed. Publish the draft with publish_training_plan, then push \
                         again to put it on the calendar."
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
                    PUSH_TOOL,
                    &format!(
                        "{} of {attempted} entries failed; first: {}",
                        report.failed.len(),
                        first.error
                    ),
                );
            }
            if report.failed.is_empty() || landed > 0 {
                emit_calendar_sync_completed(tenant, user_id, &target, PUSH_TOOL, landed);
            }
            scope.record_coach_write(PUSH_TOOL, attempted);

            ok_typed("push_training_plan", &report)
        }
        .await;
        tool_result_to_response(result)
    }
}

/// Factory for the plan-push tool set.
#[must_use]
pub fn create_training_plan_push_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![Box::new(PushTrainingPlanTool)]
}

// A push deletes calendar entries the plan no longer wants, and a delete on
// the provider cannot be undone from here — re-pushing recreates the entry,
// but as a new one. Destructive, so at most one per turn under the Guardian's
// default budget, and the confirm path applies to it when armed.
crate::declare_security!(PushTrainingPlanTool => IRREVERSIBLE);
