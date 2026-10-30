// ABOUTME: Push an athlete's active training plan to a provider calendar — render plan days into sessions, reconcile against the ledger
// ABOUTME: Provider-agnostic: speaks only FitnessProvider's calendar-write methods and the prescribed_workouts ledger; never touches the past
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Plan calendar push
//!
//! A saved plan lives in Dravr's own tables; this module is what puts it on
//! the athlete's calendar and keeps it there as the agent adjusts it.
//!
//! **Rendering.** [`crate::plan_calendar_render`] turns every non-rest plan
//! day on or after the push date into one [`PlannedSession`] and every week
//! with a focus into a week note, each under a stable [`CalendarKey`].
//!
//! **Reconciling.** Desired sessions are diffed against the ledger's live rows
//! for the same provider and against what the provider's calendar actually
//! holds: an unchanged entry (same content hash, still on the calendar) costs
//! nothing; a changed one is updated in place; a new one is created; a live
//! ledger row no plan day wants any more is deleted from the provider and
//! marked withdrawn. An entry the athlete edited on the provider since Dravr
//! last wrote it is left alone and named in the report. An entry the ledger
//! lost but the calendar still carries under Dravr's key is adopted rather than
//! duplicated. Dates before the push date are never created, updated, or
//! removed — the same rule the provider applies to its own plan changes.
//!
//! **Failure.** Every provider call is per entry, and every outcome is a ledger
//! row and a report line. A write that landed is recorded as landed even when
//! the ledger write after it fails; the next push repairs the rest. The
//! operation is idempotent by construction — re-running it is the undo.

use pierre_core::transport::TransportPolicy;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{Duration, NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{
    CalendarEventRef, CalendarKey, PlannedSession, PlannedSessionKind, PrescribedWorkout, TenantId,
};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{PlanWeek, TrainingPlan};
use pierre_providers::ai_scope;
use pierre_providers::core::FitnessProvider;
use pierre_providers::ProviderRegistry;
use serde::Serialize;
use uuid::Uuid;

use crate::plan_fuel_products::PlanFuelProducts;
use crate::plan_fueling::FuelingDisclosure;

use crate::plan_calendar_render::{desired_entries, DesiredEntry};

/// Name of the descriptor capability a push target must declare, as a refusal
/// and its `details` say it.
pub const CALENDAR_WRITE_CAPABILITY: &str = "calendar_write";

/// The provider a plan or a prescription is written to.
///
/// `requested` is the backend the caller named (already resolved from the
/// name the athlete knows), `connected` the backends the athlete holds a live
/// connection to. A named provider must declare
/// [`CALENDAR_WRITE`](pierre_providers::spi::ProviderCapabilities::CALENDAR_WRITE);
/// its connection is checked where the credentials are read. Without a name,
/// the target is the connected
/// provider that declares it, and when several qualify the first in
/// alphabetical order of backend name — so the same athlete always lands on
/// the same calendar until they name another.
///
/// # Errors
///
/// Returns an invalid-input error naming the providers that declare the
/// capability when the named one does not. Returns `NoProviderConnected`,
/// carrying `missing_capability` and the providers that would satisfy it,
/// when no connection of the athlete declares it.
pub fn resolve_calendar_target(
    registry: &ProviderRegistry,
    connected: &[&str],
    requested: Option<&str>,
) -> AppResult<String> {
    let writable = |backend: &str| {
        registry
            .get_capabilities(backend)
            .is_some_and(|caps| caps.supports_calendar_write())
    };
    if let Some(requested) = requested {
        return if writable(requested) {
            Ok(requested.to_owned())
        } else {
            Err(AppError::invalid_input(format!(
                "{requested} does not write a training calendar (capability \
                 {CALENDAR_WRITE_CAPABILITY}). Providers that do: {}",
                calendar_write_names(registry)
            )))
        };
    }
    let mut candidates: Vec<&str> = connected
        .iter()
        .copied()
        .filter(|backend| writable(backend))
        .collect();
    candidates.sort_unstable();
    candidates.dedup();
    candidates.first().map_or_else(
        || Err(missing_calendar_capability(registry)),
        |backend| Ok((*backend).to_owned()),
    )
}

/// The calendar a read of the push ledger looks under.
///
/// The connected target, else the first provider that declares the
/// capability, so a block for an athlete who never connected one is empty
/// rather than an error.
///
/// # Errors
///
/// Returns the missing-capability refusal when no registered provider
/// declares
/// [`CALENDAR_WRITE`](pierre_providers::spi::ProviderCapabilities::CALENDAR_WRITE)
/// at all.
pub fn calendar_read_target(registry: &ProviderRegistry, connected: &[&str]) -> AppResult<String> {
    resolve_calendar_target(registry, connected, None).or_else(|_| {
        registry
            .calendar_write_providers()
            .first()
            .map(|name| (*name).to_owned())
            .ok_or_else(|| missing_calendar_capability(registry))
    })
}

/// Whether `backend`'s calendar holds week notes as well as sessions
/// ([`CALENDAR_WEEK_NOTES`](pierre_providers::spi::ProviderCapabilities::CALENDAR_WEEK_NOTES)).
#[must_use]
pub fn calendar_holds_week_notes(registry: &ProviderRegistry, backend: &str) -> bool {
    registry
        .get_capabilities(backend)
        .is_some_and(|caps| caps.supports_calendar_week_notes())
}

/// The entries a calendar can hold: every session, and the week notes only
/// when it holds them — a calendar of workouts is never sent a note it would
/// refuse on every push, nor counted as behind for one.
#[must_use]
pub fn for_calendar(desired: Vec<DesiredEntry>, week_notes: bool) -> Vec<DesiredEntry> {
    if week_notes {
        return desired;
    }
    desired
        .into_iter()
        .filter(|entry| entry.session.kind != PlannedSessionKind::WeekNote)
        .collect()
}

fn calendar_write_names(registry: &ProviderRegistry) -> String {
    registry.calendar_write_providers().join(", ")
}

fn missing_calendar_capability(registry: &ProviderRegistry) -> AppError {
    let providers = registry.calendar_write_providers();
    let mut err = AppError::new(
        ErrorCode::NoProviderConnected,
        format!(
            "None of the athlete's connected providers writes a training calendar (capability \
             {CALENDAR_WRITE_CAPABILITY}). Connect one of: {}",
            providers.join(", ")
        ),
    );
    err.details = Some(Box::new(serde_json::json!({
        "action": "connect_provider",
        "missing_capability": CALENDAR_WRITE_CAPABILITY,
        "providers": providers,
    })));
    err
}

/// Seconds a provider-side `updated` stamp must trail the ledger's own write
/// by before an entry counts as edited on the provider. Absorbs clock skew
/// between this server and the provider, plus the provider stamping the change
/// a moment after it accepted ours.
const PROVIDER_EDIT_SLACK_SECONDS: i64 = 300;

/// What a push would do, judged from the ledger alone — no provider call.
///
/// This is the preview `save_training_plan` reports after a save so the
/// athlete learns the calendar is behind, and the count `get_training_plan`
/// shows next to the calendar block.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct PushPreview {
    /// Entries the ledger has no live row for.
    pub create: usize,
    /// Entries whose content differs from the live row's.
    pub update: usize,
    /// Entries whose live row already carries this content.
    pub unchanged: usize,
    /// Live rows no entry wants any more.
    pub remove: usize,
}

impl PushPreview {
    /// Whether the calendar would change at all.
    #[must_use]
    pub const fn is_stale(&self) -> bool {
        self.create + self.update + self.remove > 0
    }
}

/// Diff desired entries against the ledger's live plan rows.
///
/// # Errors
///
/// Returns an error when an entry cannot be hashed, which no well-formed
/// session produces.
pub fn diff_against_ledger(
    desired: &[DesiredEntry],
    live_rows: &[PrescribedWorkout],
) -> AppResult<PushPreview> {
    let live_by_key: HashMap<&str, &PrescribedWorkout> = live_rows
        .iter()
        .filter(|row| row.source.is_plan())
        .filter_map(|row| row.external_id.as_deref().map(|key| (key, row)))
        .collect();
    let mut preview = PushPreview::default();
    let mut wanted: HashSet<&str> = HashSet::new();
    for entry in desired {
        wanted.insert(entry.session.external_id.as_str());
        let hash = session_hash(&entry.session)?;
        match live_by_key.get(entry.session.external_id.as_str()) {
            None => preview.create += 1,
            Some(row) if row.payload_hash.as_deref() == Some(hash.as_str()) => {
                preview.unchanged += 1;
            }
            Some(_) => preview.update += 1,
        }
    }
    preview.remove = live_by_key
        .keys()
        .filter(|key| !wanted.contains(*key))
        .count();
    Ok(preview)
}

fn session_hash(session: &PlannedSession) -> AppResult<String> {
    session
        .payload_hash()
        .map_err(|e| AppError::internal(format!("hash planned session: {e}")))
}

/// Why a wanted entry was left as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// The athlete changed the entry on the provider after Dravr wrote it.
    EditedOnProvider,
}

/// An entry the push left alone, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct SkippedEntry {
    /// The entry's key.
    pub external_id: String,
    /// The entry's date.
    pub date: NaiveDate,
    /// The entry's title.
    pub name: String,
    /// Why it was skipped.
    pub reason: SkipReason,
}

/// An entry whose write the provider or the ledger refused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct FailedEntry {
    /// The entry's key.
    pub external_id: String,
    /// The entry's date.
    pub date: NaiveDate,
    /// The entry's title.
    pub name: String,
    /// What went wrong, in the provider's or the database's words.
    pub error: String,
}

/// What a push did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct PushReport {
    /// Provider the plan was pushed to.
    pub provider: String,
    /// The active plan that was pushed.
    pub plan_id: String,
    /// First date the push considered.
    pub from: NaiveDate,
    /// Last date the push considered, when there was anything to consider.
    pub to: Option<NaiveDate>,
    /// Entries created on the provider.
    pub created: usize,
    /// Entries updated in place.
    pub updated: usize,
    /// Entries already carrying this content.
    pub unchanged: usize,
    /// Entries removed from the provider because no plan day wants them.
    pub removed: usize,
    /// Entries left alone, with the reason.
    pub skipped: Vec<SkippedEntry>,
    /// Entries the provider or the ledger refused.
    pub failed: Vec<FailedEntry>,
    /// Whether a cooperative cancellation stopped the push early. Entries
    /// already written stay; the push is convergent, so a re-run reconciles
    /// the rest.
    pub cancelled: bool,
    /// Present when a human coach pushed a coached athlete's calendar while
    /// holding an unpublished draft of their plan: only the published plan
    /// was pushed, and the draft reaches the calendar once it is published
    /// and pushed again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unpublished_draft: Option<String>,
}

/// Inputs of one push.
#[derive(Debug)]
pub struct PushPlanParams<'a> {
    /// Owning tenant.
    pub tenant: TenantId,
    /// Athlete whose plan is pushed — their one season, whichever agent laid it.
    pub user_id: Uuid,
    /// Provider name the ledger rows are filed under.
    pub provider: &'a str,
    /// First date to consider — today in the athlete's calendar. Nothing
    /// before it is created, updated, or removed.
    pub from: NaiveDate,
    /// Whether the provider's calendar holds week notes
    /// ([`calendar_holds_week_notes`]); without them only sessions are pushed.
    pub week_notes: bool,
    /// Cooperative cancel flag, observed between entries. `None` for callers
    /// with no cancellation channel (the push runs to completion).
    pub cancel: Option<&'a AtomicBool>,
}

/// Whether the caller's cooperative cancel flag is raised.
fn cancel_requested(params: &PushPlanParams<'_>) -> bool {
    params
        .cancel
        .is_some_and(|flag| flag.load(Ordering::Relaxed))
}

/// Push the athlete's active plan to `calendar` and reconcile the ledger.
///
/// Cooperative cancellation: when [`PushPlanParams::cancel`] is raised the
/// push stops between entries (and before the removal pass), marks the
/// report [`PushReport::cancelled`], and returns what landed. Entries never
/// reached are simply created by the next run; entries fully written read as
/// unchanged — the reconciliation is convergent, so a partial run is always
/// recoverable by pushing again.
///
/// # Errors
///
/// Returns `UnavailableOverTransport` when the plan is stamped
/// first-party-only and the caller is an external transport. Returns an
/// error when the athlete has no active plan, or when the ledger, the
/// athlete's medical flag, the fuel catalogue or the provider's calendar
/// cannot be read at all. Per-entry write failures are reported in [`PushReport::failed`], not
/// raised.
pub async fn push_active_plan(
    repos: &RepositoryRegistry,
    calendar: &dyn FitnessProvider,
    params: &PushPlanParams<'_>,
) -> AppResult<PushReport> {
    let tenant_str = params.tenant.to_string();
    let user_str = params.user_id.to_string();
    let plan = repos
        .training_plans
        .get_active_plan(&tenant_str, &user_str)
        .await?
        .ok_or_else(|| {
            AppError::not_found(
                "no active training plan to push — build one with the athlete and save it via \
                 save_training_plan first",
            )
        })?;
    // A plan derived from first-party-only data does not leave through an
    // external transport, the same rule `get_training_plan` applies to reading
    // it: an MCP or A2A caller refused the plan must not be able to copy its
    // sessions onto a third-party calendar instead (carnet#901). A push from
    // Dravr's own surfaces to the athlete's calendar stays allowed.
    if !ai_scope::admit_derived(plan.transport_policy) {
        return Err(AppError::unavailable_over_transport(
            "this athlete's training plan is not available over this interface",
        ));
    }
    let weeks = repos
        .training_plans
        .list_plan_weeks(&tenant_str, &user_str, &plan.id, false)
        .await?;
    let fueling = FuelingDisclosure::for_athlete(repos, params.tenant, params.user_id).await?;
    let products = PlanFuelProducts::for_weeks(repos, &weeks).await?;
    let desired = for_calendar(
        desired_entries(params.user_id, &weeks, params.from, (&fueling, &products)),
        params.week_notes,
    );
    let live_rows: Vec<PrescribedWorkout> = repos
        .prescribed_workouts
        .list_live_calendar_events(
            params.tenant,
            params.user_id,
            params.provider,
            Some(params.from),
        )
        .await?
        .into_iter()
        .filter(|row| row.source.is_plan())
        .collect();

    let mut report = PushReport {
        provider: params.provider.to_owned(),
        plan_id: plan.id.clone(),
        from: params.from,
        ..PushReport::default()
    };
    let window_end = desired
        .iter()
        .map(|entry| entry.session.date)
        .chain(live_rows.iter().map(|row| row.prescribed_for_date))
        .max();
    let Some(window_end) = window_end else {
        return Ok(report);
    };
    report.to = Some(window_end);

    // What the calendar actually holds in the window, by provider id — and,
    // among those, the entries carrying this athlete's plan keys that the
    // ledger has no live row for (a write that landed while its ledger row
    // did not). Those are adopted, never duplicated. A calendar stores each
    // entry under its `calendar_key` — Dravr's key, or the opaque one a
    // provider derives from it — so the keys are compared in that form.
    let observed: HashMap<String, CalendarEventRef> = calendar
        .list_calendar_events(params.from, window_end)
        .await?
        .into_iter()
        .map(|event| (event.provider_event_id.clone(), event))
        .collect();
    let live_by_key: HashMap<String, PrescribedWorkout> = live_rows
        .iter()
        .filter_map(|row| row.external_id.clone().map(|key| (key, row.clone())))
        .collect();
    let plan_prefix = CalendarKey::plan_prefix(params.user_id);
    let adoptable: HashMap<String, String> = desired
        .iter()
        .map(|entry| &entry.session.external_id)
        .filter(|key| key.starts_with(&plan_prefix) && !live_by_key.contains_key(*key))
        .map(|key| (calendar.calendar_key(key), key.clone()))
        .collect();
    let orphans: HashMap<String, CalendarEventRef> = observed
        .values()
        .filter_map(|event| {
            let key = adoptable.get(event.external_id.as_deref()?)?;
            Some((key.clone(), event.clone()))
        })
        .collect();
    let edit_slack = Duration::seconds(PROVIDER_EDIT_SLACK_SECONDS);

    let mut wanted: HashSet<String> = HashSet::new();
    for entry in &desired {
        // Cooperative cancellation stops BETWEEN entries and must return
        // right here, never falling through to the removal pass below: the
        // `wanted` set only holds the keys of entries this loop reached, so
        // a cut-short run would read every unvisited entry's live ledger row
        // as unwanted and bulk-delete calendar entries the plan still wants.
        if cancel_requested(params) {
            report.cancelled = true;
            return Ok(report);
        }
        let key = entry.session.external_id.clone();
        wanted.insert(key.clone());
        let hash = session_hash(&entry.session)?;
        let agent = entry_author(&plan, &weeks, entry);
        let write = LedgerWrite {
            repos,
            params,
            agent,
            entry,
            hash: &hash,
            transport_policy: plan.transport_policy,
        };

        match live_by_key.get(&key) {
            Some(row) => {
                let known_event = row
                    .provider_event_id
                    .as_deref()
                    .and_then(|id| observed.get(id).map(|seen| (id.to_owned(), seen)));
                match known_event {
                    Some((event_id, seen)) => {
                        let edited = seen
                            .updated_at
                            .is_some_and(|stamp| stamp > row.updated_at + edit_slack);
                        if edited {
                            report.skipped.push(SkippedEntry {
                                external_id: key,
                                date: entry.session.date,
                                name: entry.session.name.clone(),
                                reason: SkipReason::EditedOnProvider,
                            });
                            continue;
                        }
                        if row.payload_hash.as_deref() == Some(hash.as_str()) {
                            report.unchanged += 1;
                            continue;
                        }
                        write
                            .update(calendar, &event_id, Some(row.id), &mut report)
                            .await;
                    }
                    // The ledger says live but the calendar no longer has it
                    // (the athlete deleted it): the plan still wants the day,
                    // so it is created again and the stale row superseded.
                    None => write.create(calendar, Some(row.id), &mut report).await,
                }
            }
            None => match orphans.get(&key) {
                Some(orphan) => {
                    write
                        .update(calendar, &orphan.provider_event_id, None, &mut report)
                        .await;
                }
                None => write.create(calendar, None, &mut report).await,
            },
        }
    }

    // A cancel that lands after the last entry but before the removal pass
    // still stops the push: the removal is provider-visible work too.
    if cancel_requested(params) {
        report.cancelled = true;
        return Ok(report);
    }

    // Live rows no plan day wants any more — a day that became rest, a week
    // whose dates moved. Deleted by provider id, then withdrawn in the ledger.
    let doomed: Vec<&PrescribedWorkout> = live_rows
        .iter()
        .filter(|row| {
            !row.external_id
                .as_ref()
                .is_some_and(|key| wanted.contains(key))
        })
        .collect();
    if !doomed.is_empty() {
        let ids: Vec<String> = doomed
            .iter()
            .filter_map(|row| row.provider_event_id.clone())
            .collect();
        match calendar.delete_planned_sessions(&ids).await {
            Ok(_) => {
                for row in doomed {
                    match repos
                        .prescribed_workouts
                        .set_prescribed_workout_status(
                            params.tenant,
                            row.id,
                            PrescribedWorkout::STATUS_WITHDRAWN,
                        )
                        .await
                    {
                        Ok(()) => report.removed += 1,
                        Err(e) => report.failed.push(FailedEntry {
                            external_id: row.external_id.clone().unwrap_or_default(),
                            date: row.prescribed_for_date,
                            name: row.template_slug.clone().unwrap_or_default(),
                            error: format!(
                                "the entry is gone from the calendar but the ledger row could \
                                 not be marked withdrawn: {e}"
                            ),
                        }),
                    }
                }
            }
            Err(e) => {
                for row in doomed {
                    report.failed.push(FailedEntry {
                        external_id: row.external_id.clone().unwrap_or_default(),
                        date: row.prescribed_for_date,
                        name: row.template_slug.clone().unwrap_or_default(),
                        error: format!("delete refused: {e}"),
                    });
                }
            }
        }
    }

    Ok(report)
}

/// The agent a ledger row names for `entry`: the one that wrote the plan week
/// the entry comes from — a season carries weeks written by several agents —
/// else the agent that laid the outline.
#[must_use]
pub fn entry_author<'a>(
    plan: &'a TrainingPlan,
    weeks: &'a [PlanWeek],
    entry: &DesiredEntry,
) -> Option<&'a str> {
    weeks
        .iter()
        .find(|week| week.id == entry.plan_week_id)
        .and_then(|week| week.author_agent_id.as_deref())
        .or(plan.author_agent_id.as_deref())
}

/// One entry's write, with everything needed to record its outcome.
struct LedgerWrite<'a> {
    repos: &'a RepositoryRegistry,
    params: &'a PushPlanParams<'a>,
    agent: Option<&'a str>,
    entry: &'a DesiredEntry,
    hash: &'a str,
    /// The plan's stamp (carnet#769): an entry rendered from it carries it.
    transport_policy: TransportPolicy,
}

impl LedgerWrite<'_> {
    /// Create the entry on the provider; `supersedes` is the stale live row
    /// (whose event vanished from the calendar) this write replaces.
    async fn create(
        &self,
        calendar: &dyn FitnessProvider,
        supersedes: Option<Uuid>,
        report: &mut PushReport,
    ) {
        match calendar.push_planned_session(&self.entry.session).await {
            Ok(event_id) => {
                if self.settle(Some(event_id), supersedes, report).await {
                    report.created += 1;
                }
            }
            Err(e) => {
                self.record_failure(&format!("create refused: {e}"), report)
                    .await;
            }
        }
    }

    /// Update the provider's entry `event_id` in place; `supersedes` is the
    /// live row that held it (absent for an adopted orphan).
    async fn update(
        &self,
        calendar: &dyn FitnessProvider,
        event_id: &str,
        supersedes: Option<Uuid>,
        report: &mut PushReport,
    ) {
        match calendar
            .update_planned_session(event_id, &self.entry.session)
            .await
        {
            Ok(()) => {
                if self
                    .settle(Some(event_id.to_owned()), supersedes, report)
                    .await
                {
                    report.updated += 1;
                }
            }
            Err(e) => {
                self.record_failure(&format!("update refused: {e}"), report)
                    .await;
            }
        }
    }

    /// The provider write landed: supersede the previous live row, then
    /// record the new one. Returns whether the ledger now reflects the
    /// calendar; when it does not, the report says so and names the event,
    /// because the entry IS on the calendar and the next push adopts it.
    async fn settle(
        &self,
        event_id: Option<String>,
        supersedes: Option<Uuid>,
        report: &mut PushReport,
    ) -> bool {
        if let Some(previous) = supersedes {
            if let Err(e) = self
                .repos
                .prescribed_workouts
                .set_prescribed_workout_status(
                    self.params.tenant,
                    previous,
                    PrescribedWorkout::STATUS_REPLACED,
                )
                .await
            {
                report.failed.push(self.failed_entry(format!(
                    "the entry is on the calendar (event {}) but the previous ledger row could \
                     not be superseded: {e}",
                    event_id.as_deref().unwrap_or("unknown")
                )));
                return false;
            }
        }
        match self
            .record(
                event_id.clone(),
                PrescribedWorkout::STATUS_PUSHED,
                supersedes,
            )
            .await
        {
            Ok(()) => true,
            Err(e) => {
                report.failed.push(self.failed_entry(format!(
                    "the entry is on the calendar (event {}) but its ledger row failed to save — \
                     the next push adopts it: {e}",
                    event_id.as_deref().unwrap_or("unknown")
                )));
                false
            }
        }
    }

    async fn record_failure(&self, error: &str, report: &mut PushReport) {
        if let Err(e) = self
            .record(None, PrescribedWorkout::STATUS_FAILED, None)
            .await
        {
            report.failed.push(self.failed_entry(format!(
                "{error}; and the failure could not be recorded: {e}"
            )));
            return;
        }
        report.failed.push(self.failed_entry(error.to_owned()));
    }

    fn failed_entry(&self, error: String) -> FailedEntry {
        FailedEntry {
            external_id: self.entry.session.external_id.clone(),
            date: self.entry.session.date,
            name: self.entry.session.name.clone(),
            error,
        }
    }

    async fn record(
        &self,
        provider_event_id: Option<String>,
        status: &str,
        replaces_id: Option<Uuid>,
    ) -> AppResult<()> {
        let now = Utc::now();
        let payload_json = serde_json::to_string(&self.entry.session)
            .map_err(|e| AppError::internal(format!("serialize planned session: {e}")))?;
        let row = PrescribedWorkout {
            id: Uuid::new_v4(),
            tenant_id: self.params.tenant.as_uuid(),
            user_id: self.params.user_id,
            agent_id: self.agent.map(str::to_owned),
            template_slug: None,
            sport: self.entry.session.sport.clone(),
            prescribed_for_date: self.entry.session.date,
            provider: self.params.provider.to_owned(),
            provider_event_id,
            external_id: Some(self.entry.session.external_id.clone()),
            source: self.entry.source,
            plan_week_id: Some(self.entry.plan_week_id.clone()),
            replaces_id,
            payload_hash: Some(self.hash.to_owned()),
            payload_json,
            status: status.to_owned(),
            created_at: now,
            updated_at: now,
            transport_policy: self.transport_policy,
        };
        self.repos
            .prescribed_workouts
            .upsert_prescribed_workout(&row)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_providers::spi::{
        OAuthEndpoints, OAuthParams, OAuthRefresh, ProviderCapabilities, ProviderDescriptor,
    };

    /// A calendar-writing provider the reconciler has never heard of.
    struct MockCalendar {
        name: &'static str,
        capabilities: ProviderCapabilities,
    }

    impl ProviderDescriptor for MockCalendar {
        fn name(&self) -> &'static str {
            self.name
        }
        fn display_name(&self) -> &'static str {
            self.name
        }
        fn capabilities(&self) -> ProviderCapabilities {
            self.capabilities
        }
        fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
            None
        }
        fn oauth_params(&self) -> Option<OAuthParams> {
            None
        }
        fn oauth_refresh(&self) -> Option<OAuthRefresh> {
            None
        }
        fn api_base_url(&self) -> &'static str {
            ""
        }
        fn default_scopes(&self) -> &'static [&'static str] {
            &[]
        }
    }

    fn registry() -> ProviderRegistry {
        let mut registry = ProviderRegistry::new();
        for (name, capabilities) in [
            (
                "mock_calendar_b",
                ProviderCapabilities::ACTIVITIES.union(ProviderCapabilities::CALENDAR_WRITE),
            ),
            (
                "mock_calendar_a",
                ProviderCapabilities::ACTIVITIES.union(ProviderCapabilities::CALENDAR_WRITE),
            ),
            (
                "mock_read_only",
                ProviderCapabilities::ACTIVITIES.union(ProviderCapabilities::PLANNED_WORKOUTS),
            ),
        ] {
            registry.register_descriptor(name, Box::new(MockCalendar { name, capabilities }));
        }
        registry
    }

    #[test]
    fn the_target_is_the_connected_provider_that_writes_a_calendar() {
        let target = resolve_calendar_target(&registry(), &["strava", "mock_calendar_b"], None);
        assert_eq!(target.unwrap(), "mock_calendar_b");
    }

    #[test]
    fn several_qualifying_connections_resolve_alphabetically_whatever_the_order() {
        let registry = registry();
        for connected in [
            ["mock_calendar_b", "mock_calendar_a"],
            ["mock_calendar_a", "mock_calendar_b"],
        ] {
            let target = resolve_calendar_target(&registry, &connected, None);
            assert_eq!(target.unwrap(), "mock_calendar_a");
        }
    }

    #[test]
    fn a_named_provider_wins_over_the_default_order() {
        let target = resolve_calendar_target(
            &registry(),
            &["mock_calendar_a", "mock_calendar_b"],
            Some("mock_calendar_b"),
        );
        assert_eq!(target.unwrap(), "mock_calendar_b");
    }

    #[test]
    fn a_named_provider_without_the_capability_is_refused_by_name() {
        let err = resolve_calendar_target(&registry(), &[], Some("mock_read_only")).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
        assert!(
            err.message.contains(CALENDAR_WRITE_CAPABILITY),
            "{}",
            err.message
        );
        assert!(err.message.contains("mock_calendar_a"), "{}", err.message);
    }

    #[test]
    fn no_calendar_capable_connection_names_the_missing_capability() {
        let err =
            resolve_calendar_target(&registry(), &["strava", "mock_read_only"], None).unwrap_err();
        assert_eq!(err.code, ErrorCode::NoProviderConnected);
        assert!(
            err.message.contains(CALENDAR_WRITE_CAPABILITY),
            "{}",
            err.message
        );
        let details = err.details.expect("details");
        assert_eq!(details["missing_capability"], CALENDAR_WRITE_CAPABILITY);
        let providers = details["providers"].as_array().expect("providers");
        assert!(providers.iter().any(|name| name == "mock_calendar_a"));
        assert!(providers.iter().all(|name| name != "mock_read_only"));
    }

    #[test]
    fn a_read_looks_under_the_first_calendar_provider_when_none_is_connected() {
        // The registry also holds whichever real calendar providers this
        // build compiles (intervals_icu under --all-features), so the first
        // in alphabetical order is read from it rather than assumed.
        let registry = registry();
        let first = registry.calendar_write_providers()[0];
        assert!(first <= "mock_calendar_a", "{first}");
        let target = calendar_read_target(&registry, &["strava"]);
        assert_eq!(target.unwrap(), first);
    }

    #[test]
    fn a_read_prefers_the_connected_target() {
        let target = calendar_read_target(&registry(), &["mock_calendar_b"]);
        assert_eq!(target.unwrap(), "mock_calendar_b");
    }
}
