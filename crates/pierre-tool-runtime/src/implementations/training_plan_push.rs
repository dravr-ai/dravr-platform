// ABOUTME: push_training_plan tool — puts the athlete's active plan on their provider calendar and reconciles the ledger
// ABOUTME: Also the push a coach's publish ends with, the athlete's own « Envoyer » and its review, the calendar block and the publish's preview
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::annotations::{annotations, Effect, Reach};
use crate::tool_data::{ToolData, WriteKind};
use dravr_barrage::Stream;
use pierre_providers::ai_scope;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use dravr_tronc::dates::CivilDate;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{PrescribedWorkout, TenantId};
use pierre_database::repositories::{PlanChangeKind, PlanChangeSource};
use pierre_memory::training_plans::{parse_plan_date, PlanWeek};
use pierre_services::athlete_clock::athlete_today;
use pierre_services::coach_plan_draft::{draft_state, CoachDraftTarget};
use pierre_services::coach_plan_notice::{tell_plan_pushed, CoachCalendarPush};
use pierre_services::own_plan::clear_sent_marks;
use pierre_services::plan_calendar_push::{
    calendar_holds_week_notes, desired_entries, diff_against_ledger, edited_on_calendar,
    for_calendar, push_active_plan, KeyDiff, LedgerDiff, PushPlanParams, PushReport,
};
use pierre_services::plan_change_history::{record_plan_change, PlanChange};
use pierre_services::plan_fueling::FuelingDisclosure;
use serde::Serialize;
use serde_json::{json, Value};
use tracing::warn;
use uuid::Uuid;

use super::calendar::{
    calendar_ledger_target, holds_only_through_a_link, provider_arg, provider_property,
    CalendarConnections, LedgerCalendar, PassedOver,
};
use super::coach_scope::{
    athlete_property, resolve_calendar_scope, under_athlete_consents, CalendarScope,
};
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

/// The athlete's plan weeks against Dravr's push ledger on `backend`, from
/// `today` on: the live rows the caller may read, and the per-key diff a
/// push there would act on.
#[derive(Debug)]
pub struct LedgerRead {
    /// The ledger's live rows on the calendar from `today` on, plan days and
    /// single prescriptions alike, those an external caller may not read
    /// left out (carnet#769).
    pub live: Vec<PrescribedWorkout>,
    /// The plan's wanted entries against the live plan rows, key by key.
    pub diff: LedgerDiff,
}

/// The one diff of `weeks` against the ledger on `backend` (carnet#1841).
///
/// The calendar block, the publish's preview and the athlete's « Envoyer »
/// count all read it, so none of them recomputes it its own way.
///
/// The days render with the push's own disclosure
/// ([`FuelingDisclosure::for_athletes_calendar`]), so the pending entries are
/// what the push would write whoever reads them, and a calendar that holds no
/// week notes is never counted behind for one.
///
/// # Errors
///
/// Returns an error when the ledger or the athlete's medical flag cannot be
/// read.
pub async fn ledger_diff(
    context: &ToolExecutionContext,
    tenant: TenantId,
    user_id: Uuid,
    backend: &str,
    weeks: &[PlanWeek],
    today: CivilDate,
) -> AppResult<LedgerRead> {
    let repos = context.resources.repos();
    let fueling = FuelingDisclosure::for_athletes_calendar(repos, tenant, user_id).await?;
    let mut live = repos
        .prescribed_workouts
        .list_live_calendar_events(tenant, user_id, backend, Some(today))
        .await?;
    // An entry pushed from first-party-only data is withheld from an external
    // caller; the rest are served (carnet#769).
    ai_scope::retain_admitted(&mut live, |row| row.transport_policy);
    let desired = for_calendar(
        desired_entries(user_id, weeks, today, &fueling),
        calendar_holds_week_notes(context.resources.provider_registry(), backend),
    );
    let diff = diff_against_ledger(&desired, &live)?;
    Ok(LedgerRead { live, diff })
}

/// Why the athlete's own « Envoyer » is not offered (carnet#1841).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SendBlocked {
    /// None of the athlete's connections writes a calendar: there is
    /// nowhere to send.
    NoProvider,
    /// Several of their connections write a calendar and none is chosen for
    /// Dravr's pushes: they choose one first.
    ChooseCalendar,
    /// The calendar is theirs only through a human coach's link: only that
    /// coach writes it.
    CoachManaged,
}

impl SendBlocked {
    /// The reason as the wire carries it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoProvider => "no_provider",
            Self::ChooseCalendar => "choose_calendar",
            Self::CoachManaged => "coach_managed",
        }
    }
}

/// What the athlete's own « Envoyer » would send, judged from the ledger
/// alone — no provider call (carnet#1841).
#[derive(Debug, Clone, Default)]
pub struct PendingSend {
    /// The calendar backend the send would write; `None` when there is no
    /// calendar to send to, or the athlete must choose one first.
    pub destination: Option<String>,
    /// Why the send is not offered; `None` when it is.
    pub blocked: Option<SendBlocked>,
    /// The calendars to choose among, when the athlete must choose one.
    pub calendars: Vec<String>,
    /// One entry per key the send would create, update or withdraw on
    /// `destination` ([`ledger_diff`]); empty when there is no destination.
    pub entries: Vec<KeyDiff>,
}

impl PendingSend {
    /// Whether the athlete may send now.
    #[must_use]
    pub const fn send_available(&self) -> bool {
        self.blocked.is_none()
    }
}

/// The athlete's own calendar scope, as their own `push_training_plan` with
/// no argument resolves it.
///
/// # Errors
///
/// Returns the lookup errors of the resolution, and an internal error for a
/// refusal, which a call on the caller's own plan with no conversation never
/// meets.
async fn own_calendar_scope(context: &ToolExecutionContext) -> AppResult<CalendarScope> {
    match resolve_calendar_scope(context, &json!({}), None, PUSH_TOOL).await? {
        Ok(scope) => Ok(scope),
        Err(refused) => Err(AppError::internal(format!(
            "the athlete's own calendar scope was refused: {}",
            refused.content
        ))),
    }
}

/// What the athlete's own « Envoyer » would send of `weeks`, their live plan,
/// from `today` on, and whether they may send it (carnet#1841).
///
/// The destination is the calendar their own push resolves to. With no
/// calendar to write, or several and none chosen, nothing is pending; one
/// held only through a human coach's link is diffed but not theirs to send.
/// Never a provider call: what the athlete edited on the provider is the send
/// review's to find.
///
/// # Errors
///
/// Returns the lookup errors of the calendar's resolution and of
/// [`ledger_diff`].
pub async fn own_pending_send(
    context: &ToolExecutionContext,
    weeks: &[PlanWeek],
    today: CivilDate,
) -> AppResult<PendingSend> {
    let scope = own_calendar_scope(context).await?;
    let (tenant, user_id) = (scope.tenant, scope.user_id);
    let target = match scope.target(context, None).await {
        Ok(Ok(target)) => target,
        Ok(Err(choose)) => {
            return Ok(PendingSend {
                blocked: Some(SendBlocked::ChooseCalendar),
                calendars: offered_calendars(&choose),
                ..PendingSend::default()
            })
        }
        Err(error) if error.code == ErrorCode::NoProviderConnected => {
            return Ok(PendingSend {
                blocked: Some(SendBlocked::NoProvider),
                ..PendingSend::default()
            })
        }
        Err(error) => return Err(error),
    };
    let backend = target.backend;
    let coach_managed = holds_only_through_a_link(context, tenant, user_id, &backend).await?;
    let read = ledger_diff(context, tenant, user_id, &backend, weeks, today).await?;
    Ok(PendingSend {
        entries: read.diff.pending().cloned().collect(),
        destination: Some(backend),
        blocked: coach_managed.then_some(SendBlocked::CoachManaged),
        calendars: Vec::new(),
    })
}

/// What the notify events name as the trigger of the athlete's own send.
const OWN_SEND_TRIGGER: &str = "own_plan_send";

/// Why the athlete's own send, or its review, was refused (carnet#1841).
/// Nothing on their calendar was read or written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnSendRefusal {
    /// None of their connections writes a calendar.
    NoCalendar,
    /// Several of their connections write a calendar and none is chosen:
    /// the calendars to choose among.
    ChooseCalendar(Vec<String>),
    /// The calendar is theirs only through a human coach's link, whom it is
    /// for to write.
    CoachManaged,
    /// Their tenant switched `push_training_plan` off.
    TenantDisabled,
    /// They have no plan to send.
    NoPlan,
    /// Another gate refused it, with its own reason and words: the Garmin
    /// send consent, a grant the provider still owes.
    Refused {
        /// The gate's reason.
        reason: String,
        /// The gate's words.
        message: String,
    },
}

impl OwnSendRefusal {
    /// The `details.reason` a client branches on.
    #[must_use]
    pub fn reason(&self) -> &str {
        match self {
            Self::NoCalendar => "no_calendar",
            Self::ChooseCalendar(_) => "choose_calendar",
            Self::CoachManaged => "coach_managed",
            Self::TenantDisabled => TENANT_DISABLED_REASON,
            Self::NoPlan => "no_plan",
            Self::Refused { reason, .. } => reason,
        }
    }

    /// The refusal in words.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::NoCalendar => {
                "None of your connections takes workouts: connect a calendar to send your plan"
                    .to_owned()
            }
            Self::ChooseCalendar(_) => {
                "Several of your calendars take workouts: choose the one Dravr sends to".to_owned()
            }
            Self::CoachManaged => {
                "Your calendar is written by your human coach, through their link".to_owned()
            }
            Self::TenantDisabled => tenant_disabled_message(PUSH_TOOL),
            Self::NoPlan => "You have no plan to send yet".to_owned(),
            Self::Refused { message, .. } => message.clone(),
        }
    }

    /// The refusal a blocked pending send names.
    fn of_blocked(pending: &PendingSend) -> Option<Self> {
        pending.blocked.map(|blocked| match blocked {
            SendBlocked::NoProvider => Self::NoCalendar,
            SendBlocked::ChooseCalendar => Self::ChooseCalendar(pending.calendars.clone()),
            SendBlocked::CoachManaged => Self::CoachManaged,
        })
    }

    /// A gate's in-band refusal of the push.
    fn of_gate(refused: &ToolResult) -> Self {
        let field = |key: &str| refused.content.get(key).and_then(Value::as_str);
        match field("reason") {
            Some("choose_calendar") => Self::ChooseCalendar(offered_calendars(refused)),
            Some("human_coach_writes_calendar") => Self::CoachManaged,
            reason => Self::Refused {
                reason: reason.unwrap_or(COACHING_GATE).to_owned(),
                message: field("error").map_or_else(|| refused.content.to_string(), str::to_owned),
            },
        }
    }
}

/// What « Envoyer » would send, checked against the provider (carnet#1841).
#[derive(Debug, Clone)]
pub struct SendReview {
    /// What the send would change, judged from the ledger.
    pub pending: PendingSend,
    /// The keys of `pending` whose entry the athlete changed on the provider
    /// after Dravr wrote it: the send leaves them alone unless the athlete
    /// replaces them.
    pub edited_on_provider: Vec<String>,
}

/// Review what the athlete's own « Envoyer » would send of `weeks` from `today` on.
///
/// `weeks` is their live plan. The review is [`own_pending_send`], with each
/// update whose
/// entry the athlete changed on the provider since Dravr wrote it named
/// (decision 3, Phil 2026-11-26). Reads the provider's calendar; writes
/// nothing.
///
/// # Errors
///
/// The lookup errors of [`own_pending_send`], of building the provider and of
/// reading its calendar.
pub async fn review_own_send(
    context: &ToolExecutionContext,
    weeks: &[PlanWeek],
    today: CivilDate,
) -> AppResult<Result<SendReview, OwnSendRefusal>> {
    let pending = own_pending_send(context, weeks, today).await?;
    if let Some(refused) = OwnSendRefusal::of_blocked(&pending) {
        return Ok(Err(refused));
    }
    let Some(destination) = pending.destination.clone() else {
        return Ok(Err(OwnSendRefusal::NoCalendar));
    };
    let mut scope = own_calendar_scope(context).await?;
    let calendar = match scope.calendar_writer(context, &destination).await? {
        Ok(calendar) => calendar,
        Err(refused) => return Ok(Err(OwnSendRefusal::of_gate(&refused))),
    };
    let (tenant, user_id) = (scope.tenant, scope.user_id);
    let rows = context
        .resources
        .repos()
        .prescribed_workouts
        .list_live_calendar_events(tenant, user_id, &destination, Some(today))
        .await?;
    let edited = edited_on_calendar(calendar.as_ref(), &rows, &pending.entries).await?;
    Ok(Ok(SendReview {
        pending,
        edited_on_provider: edited,
    }))
}

/// The athlete's own « Envoyer » (carnet#1841).
///
/// Their live plan is put on their calendar by the push `push_training_plan`
/// runs, its every gate first, overwriting the entries they edited on the
/// provider only on the dates `replace_edited` names. The push records the
/// sessions it wrote as the athlete's `send` and clears their editor marks,
/// as it does for the agent's push on the athlete's yes.
///
/// # Errors
///
/// The lookup errors of the gates and of the push.
pub async fn send_own_plan(
    context: &ToolExecutionContext,
    replace_edited: &[CivilDate],
) -> AppResult<Result<PushReport, OwnSendRefusal>> {
    let tenant = TenantId::from_uuid(context.require_tenant()?);
    if !caller_tool_enabled(&context.resources, tenant, context.user_id, PUSH_TOOL).await {
        return Ok(Err(OwnSendRefusal::TenantDisabled));
    }
    let pushed = Box::pin(push_active_plan_for(
        context,
        &json!({}),
        OWN_SEND_TRIGGER,
        AthleteNotice::Tell,
        replace_edited,
    ))
    .await;
    let report = match pushed {
        Ok(PlanPush::Pushed(report)) => report,
        Ok(PlanPush::Refused(refused)) => return Ok(Err(OwnSendRefusal::of_gate(&refused))),
        Err(error) if error.code == ErrorCode::NoProviderConnected => {
            return Ok(Err(OwnSendRefusal::NoCalendar))
        }
        Err(error) if error.code == ErrorCode::ResourceNotFound => {
            return Ok(Err(OwnSendRefusal::NoPlan))
        }
        Err(error) => return Err(error),
    };
    Ok(Ok(report))
}

/// What the athlete's own send leaves once it wrote their calendar
/// (carnet#1841): the dates it sent recorded as their `send`, and the editor
/// marks on those dates cleared (Phil, 2026-11-27).
///
/// Best-effort: the calendar write has landed, so a mark that cannot be
/// cleared is logged, never reported as a failed send.
async fn own_send_landed(
    context: &ToolExecutionContext,
    tenant: TenantId,
    athlete: Uuid,
    report: &PushReport,
) {
    let mut dates = report.sent_dates.clone();
    dates.sort_unstable();
    dates.dedup();
    if dates.is_empty() {
        return;
    }
    let repos = context.resources.repos();
    let change = PlanChange {
        source: PlanChangeSource::Athlete,
        kind: PlanChangeKind::Send,
        dates: &dates,
    };
    record_plan_change(repos, &CoachDraftTarget::own(tenant, athlete), change).await;
    if let Err(error) = clear_sent_marks(repos, tenant, athlete, &dates).await {
        warn!(
            error = %error,
            user_id = %athlete,
            "own send: the sent dates' editor marks could not be cleared"
        );
    }
}

/// The calendars a `choose_calendar` refusal offers, as the push names them.
fn offered_calendars(choose: &ToolResult) -> Vec<String> {
    choose
        .content
        .get("calendars")
        .and_then(Value::as_array)
        .map_or_default(|offered| {
            offered
                .iter()
                .filter_map(|calendar| calendar.get("provider").and_then(Value::as_str))
                .map(str::to_owned)
                .collect()
        })
}

/// The calendar block: every entry Dravr has live on the athlete's calendar
/// from `today` on, and what a push would change.
///
/// Each entry carries its `prescription_id` so a later turn can name it to
/// `prescribe_workout`'s `replaces` or to `withdraw_prescribed_workout`, and
/// its `source` so the model knows which entries belong to the plan instead.
/// What a push would change is [`ledger_diff`]'s. `scope` is whose plan it is
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
    today: CivilDate,
) -> AppResult<CalendarBlock> {
    let (tenant, user_id) = (scope.tenant, scope.user_id);
    let ledger =
        calendar_ledger_target(context, tenant, user_id, ledger_connections(scope)).await?;
    let pushed_by = who_pushes(scope, &ledger);
    let calendar_note = read_note(context, scope, &ledger).await?;
    let connected = ledger.connected;
    let target = ledger.backend;
    let LedgerRead { live, diff } =
        ledger_diff(context, tenant, user_id, &target, active_weeks, today).await?;
    // Each entry is a plan day or a prescription an agent wrote, and a result
    // serving one is labelled so.
    if !live.is_empty() {
        ai_scope::note_agent_written();
    }
    let pending = diff.preview();
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
                date: row.prescribed_for_date.to_string(),
                source: row.source,
                name,
                provider_event_id: row.provider_event_id.clone(),
                pushed_at: row.updated_at,
            }
        })
        .collect();
    // A calendar nothing backs is said so beside its name, unless the note
    // already names the calendar the athlete chose and why it was passed over.
    let calendar_note = calendar_note.or_else(|| {
        (!connected).then(|| {
            format!(
                "{target} is not connected for this athlete, so it is not their active \
                 calendar: what is listed here is only what Dravr pushed there before, and \
                 push_training_plan writes nothing there until a calendar is connected."
            )
        })
    });
    let mut block = CalendarBlock {
        provider: target,
        connected,
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

/// What a push would change, for a coach's `publish_training_plan` reply
/// when its push did not run — only once the calendar already carries this
/// plan, so an athlete who never pushed is not nagged about a calendar they
/// do not use.
///
/// Best-effort like the ramp check: the plan is already committed, so an
/// unreadable ledger degrades to `None` rather than failing the publish.
/// What a push would change is [`ledger_diff`]'s, and `scope` says who
/// pushes it, as [`calendar_block`] does.
pub(super) async fn calendar_preview_after_save(
    context: &ToolExecutionContext,
    scope: &PlanScope,
    plan_id: &str,
    today: CivilDate,
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
    let weeks = best_effort(
        repos
            .training_plans
            .list_plan_weeks(&tenant.to_string(), &user_id.to_string(), plan_id, false)
            .await,
        "plan weeks unreadable for the calendar preview",
    )?;
    let read = best_effort(
        ledger_diff(context, tenant, user_id, &target, &weeks, today).await,
        "calendar preview could not be computed",
    )?;
    if !read.live.iter().any(|row| row.source.is_plan()) {
        return None;
    }
    let pending = read.diff.preview();
    Some(CalendarPreview {
        provider: target,
        pending,
        stale: pending.is_stale(),
        pushed_by,
        calendar_note,
    })
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
             dates before today. Call it only on the athlete's yes to sending, or a coach's ask: \
             a save never sends, and publish_training_plan pushes a coach's publish itself \
             (its calendar_push says what landed). Needs a saved plan and a connected \
             calendar-writing provider. In Dravr's app or channels a group's human coach can push a \
             consenting athlete's plan through their Intervals.icu link: they must allow \
             calendar writes on it, and Intervals.icu must list the coach as their coach; \
             elsewhere a push is the caller's own. Only the published plan is pushed, never a coach's \
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
        let result = push_active_plan_for(&context, &args, PUSH_TOOL, AthleteNotice::Tell, &[])
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
/// The one body behind `push_training_plan` and a coach's publish
/// (carnet#1492). `args` is read as the tool takes it:
/// `athlete`, `provider` and `from_date`. The calendar scope's gates
/// ([`resolve_calendar_scope`]) run on every call under the tool's name, so a
/// coach writes an athlete's calendar only on Dravr's own surfaces and
/// outside a room whatever triggered the write, and the athlete's tenant is
/// held to its own `push_training_plan` setting. The athlete is told when
/// their coach wrote their calendar, as `notice` says; the notify events
/// name `trigger`. `replace_edited` names the dates whose entry the athlete
/// edited on the provider that the push overwrites anyway.
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
    replace_edited: &[CivilDate],
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
        let provider = match scope.calendar_sender(context, &target).await? {
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
                replace_edited,
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
        if scope.athlete_name().is_none() {
            Box::pin(own_send_landed(context, tenant, user_id, &report)).await;
        }
        Ok(PlanPush::Pushed(report))
    })
    .await
}

/// The `reason` reported for a refusal by the plan scope's coaching gates,
/// which name none of their own: the athlete is not one the requester may
/// act for from here.
const COACHING_GATE: &str = "coaching_gate";

/// Put the athlete's active plan on their chosen calendar once a coach's
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
    match push_active_plan_for(context, args, trigger, notice, &[]).await {
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
crate::declare_security!(
    PushTrainingPlanTool => IRREVERSIBLE,
    reads: ToolData::own(&[Stream::Workout])
        .writes(WriteKind::Structured),
);
