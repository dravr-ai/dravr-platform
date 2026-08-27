// ABOUTME: Push an athlete's active training plan to a provider calendar — render plan days into sessions, reconcile against the ledger
// ABOUTME: Provider-agnostic: speaks only FitnessProvider's calendar-write methods and the prescribed_workouts ledger; never touches the past
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Plan calendar push
//!
//! A saved plan lives in Dravr's own tables; this module is what puts it on
//! the athlete's calendar and keeps it there as the coach adjusts it.
//!
//! **Rendering.** Every non-rest [`PlannedDay`] on or after the push date
//! becomes one [`PlannedSession`] keyed by [`CalendarKey::plan_day`]; a plan
//! week with a focus becomes one week note keyed by
//! [`CalendarKey::plan_week_note`]. The mapping is deterministic and holds no
//! model in the loop: the coach's prose goes out verbatim, and a day gets a
//! single structured step only when its `intensity` is inside
//! [`RelativeIntensity`]'s grammar — otherwise it is a timed entry carrying the
//! coach's words, never a step the coach did not state.
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

use std::collections::{HashMap, HashSet};

use chrono::{Duration, NaiveDate, Utc};
use pierre_core::constants::oauth::INTERVALS_ICU;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    CalendarEventRef, CalendarEventSource, CalendarKey, PlannedSession, PlannedSessionKind,
    PrescribedWorkout, RelativeIntensity, SportType, TenantId, WorkoutStep,
};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{parse_plan_date, PlanWeek, PlannedDay};
use pierre_providers::core::FitnessProvider;
use serde::Serialize;
use uuid::Uuid;

/// The provider whose calendar plans and prescriptions are written to.
///
/// Intervals.icu is the only connected backend with a writable training
/// calendar; every other provider inherits the trait defaults, which report
/// the capability as unsupported.
pub const CALENDAR_PROVIDER: &str = INTERVALS_ICU;

/// Seconds a provider-side `updated` stamp must trail the ledger's own write
/// by before an entry counts as edited on the provider. Absorbs clock skew
/// between this server and the provider, plus the provider stamping the change
/// a moment after it accepted ours.
const PROVIDER_EDIT_SLACK_SECONDS: i64 = 300;

/// Longest title a plan day gets on the calendar.
const MAX_TITLE_CHARS: usize = 60;

/// Sport for a plan day's free-text sport label.
///
/// Coaches write sports the way they say them, in French or English; the
/// aliases here are the ones the alpha plans used, on top of the canonical
/// snake-case names [`SportType::from_internal_string`] already reads. An
/// unknown label is kept as [`SportType::Other`] so the provider files it as
/// a generic workout rather than guessing a discipline.
#[must_use]
pub fn plan_sport(label: &str) -> SportType {
    let lowered = label.trim().to_lowercase();
    match lowered.as_str() {
        "bike" | "ride" | "cycling" | "vélo" | "velo" | "road" | "route" | "bike_ride" => {
            SportType::Ride
        }
        "mtb" | "mountain bike" | "vtt" | "xco" => SportType::MountainBike,
        "gravel" => SportType::GravelRide,
        "trainer" | "indoor" | "zwift" | "home trainer" | "virtual ride" => SportType::VirtualRide,
        "run" | "running" | "course" | "course à pied" | "jog" | "jogging" => SportType::Run,
        "trail" | "trail run" | "trail running" => SportType::TrailRunning,
        "swim" | "swimming" | "natation" | "piscine" => SportType::Swim,
        "walk" | "walking" | "marche" => SportType::Walk,
        "hike" | "hiking" | "rando" | "randonnée" => SportType::Hike,
        "strength" | "gym" | "muscu" | "musculation" | "weights" | "force" | "renfo" => {
            SportType::StrengthTraining
        }
        "ski" | "xc ski" | "ski de fond" | "nordic" | "nordic ski" => SportType::CrossCountrySkiing,
        "row" | "rowing" | "aviron" => SportType::Rowing,
        other => SportType::from_internal_string(&other.replace(' ', "_")),
    }
}

/// Truncate to `max` characters, marking the cut with an ellipsis.
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// The calendar title of a plan day: the first clause of the coach's
/// prescription, or the sport when the prescription is empty.
fn day_title(workout: &str, sport: &SportType) -> String {
    let clause = workout
        .split(['.', ';', ':', '\n'])
        .next()
        .unwrap_or_default();
    let clause = clause.split(" — ").next().unwrap_or(clause);
    let clause = clause.split(" – ").next().unwrap_or(clause).trim();
    if clause.is_empty() {
        return sport.display_name().to_owned();
    }
    truncate_chars(clause, MAX_TITLE_CHARS)
}

/// Render one plan day as a calendar session, or `None` for a rest day or a
/// day whose date is not a plan date.
#[must_use]
pub fn plan_day_session(user_id: Uuid, day: &PlannedDay, ordinal: usize) -> Option<PlannedSession> {
    if day.is_rest() {
        return None;
    }
    let date = parse_plan_date(&day.date)?;
    let sport = plan_sport(&day.sport);
    let name = day_title(&day.workout, &sport);
    let intensity = day.intensity.trim();
    let mut notes = day.workout.trim().to_owned();
    if !intensity.is_empty() {
        if !notes.is_empty() {
            notes.push('\n');
        }
        notes.push_str(intensity);
    }
    let duration_seconds = day
        .duration_min
        .filter(|minutes| *minutes > 0)
        .map(|minutes| minutes.saturating_mul(60));
    // One structured step only when the coach stated an intensity Dravr can
    // express as a target; the sport name is the cue because a cue drawn from
    // the prose could carry a "2h" the provider's parser would read as a
    // duration.
    let steps = match (duration_seconds, RelativeIntensity::parse(intensity)) {
        (Some(seconds), Some(_)) => vec![WorkoutStep {
            label: sport.display_name().to_owned(),
            duration_seconds: seconds,
            distance_meters: None,
            target_zone: intensity.to_owned(),
            repeat: 1,
            note: None,
        }],
        _ => Vec::new(),
    };
    Some(PlannedSession {
        external_id: CalendarKey::plan_day(user_id, date, ordinal),
        kind: PlannedSessionKind::Workout,
        date,
        sport,
        name,
        duration_seconds,
        notes,
        steps,
    })
}

/// Render a plan week's focus as a week note, or `None` when the week has
/// no focus to pin.
#[must_use]
pub fn week_note_session(
    user_id: Uuid,
    week: &PlanWeek,
    week_start: NaiveDate,
) -> Option<PlannedSession> {
    let focus = week.focus.trim();
    if focus.is_empty() {
        return None;
    }
    let mut notes = focus.to_owned();
    let reason = week.adjustment_reason.trim();
    if !reason.is_empty() {
        notes.push('\n');
        notes.push_str(reason);
    }
    Some(PlannedSession {
        external_id: CalendarKey::plan_week_note(user_id, week_start),
        kind: PlannedSessionKind::WeekNote,
        date: week_start,
        sport: SportType::Workout,
        name: truncate_chars(focus, MAX_TITLE_CHARS),
        duration_seconds: None,
        notes,
        steps: Vec::new(),
    })
}

/// One calendar entry the plan wants, with the week it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct DesiredEntry {
    /// The entry as the provider will receive it.
    pub session: PlannedSession,
    /// The active plan week the entry was rendered from.
    pub plan_week_id: String,
    /// Plan day or week note.
    pub source: CalendarEventSource,
}

/// Every entry the plan wants on the calendar on or after `from`.
///
/// Two sessions on one date get ordinals 0 and 1 in the week's own order, so
/// a brick day is two entries with two stable keys. A week note is wanted only
/// for weeks starting on or after `from`: a week already under way is partly
/// past, and the past is not rewritten.
#[must_use]
pub fn desired_entries(user_id: Uuid, weeks: &[PlanWeek], from: NaiveDate) -> Vec<DesiredEntry> {
    let mut out = Vec::new();
    for week in weeks {
        let Some(week_start) = parse_plan_date(&week.week_start) else {
            continue;
        };
        if week_start >= from {
            if let Some(note) = week_note_session(user_id, week, week_start) {
                out.push(DesiredEntry {
                    session: note,
                    plan_week_id: week.id.clone(),
                    source: CalendarEventSource::PlanWeekNote,
                });
            }
        }
        let mut ordinals: HashMap<NaiveDate, usize> = HashMap::new();
        for day in &week.days {
            if day.is_rest() {
                continue;
            }
            let Some(date) = parse_plan_date(&day.date) else {
                continue;
            };
            let ordinal = ordinals.entry(date).or_insert(0);
            let this_ordinal = *ordinal;
            *ordinal += 1;
            if date < from {
                continue;
            }
            if let Some(session) = plan_day_session(user_id, day, this_ordinal) {
                out.push(DesiredEntry {
                    session,
                    plan_week_id: week.id.clone(),
                    source: CalendarEventSource::PlanDay,
                });
            }
        }
    }
    out
}

/// What a push would do, judged from the ledger alone — no provider call.
///
/// This is the preview `save_training_plan` reports after a save so the
/// athlete learns the calendar is behind, and the count `get_training_plan`
/// shows next to the calendar block.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// The athlete changed the entry on the provider after Dravr wrote it.
    EditedOnProvider,
}

/// An entry the push left alone, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
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
}

/// Inputs of one push.
pub struct PushPlanParams<'a> {
    /// Owning tenant.
    pub tenant: TenantId,
    /// Athlete whose plan is pushed.
    pub user_id: Uuid,
    /// Coach persona whose plan to resolve; `None` for the coach-agnostic one.
    pub coach_slug: Option<&'a str>,
    /// Provider name the ledger rows are filed under.
    pub provider: &'a str,
    /// First date to consider — today in the athlete's calendar. Nothing
    /// before it is created, updated, or removed.
    pub from: NaiveDate,
}

/// Push the athlete's active plan to `calendar` and reconcile the ledger.
///
/// # Errors
///
/// Returns an error when the athlete has no active plan, or when the ledger or
/// the provider's calendar cannot be read at all. Per-entry write failures
/// are reported in [`PushReport::failed`], not raised.
pub async fn push_active_plan(
    repos: &RepositoryRegistry,
    calendar: &dyn FitnessProvider,
    params: &PushPlanParams<'_>,
) -> AppResult<PushReport> {
    let tenant_str = params.tenant.to_string();
    let user_str = params.user_id.to_string();
    let plan = repos
        .training_plans
        .get_active_plan(&tenant_str, &user_str, params.coach_slug)
        .await?
        .ok_or_else(|| {
            AppError::not_found(
                "no active training plan to push — build one with the athlete and save it via \
                 save_training_plan first",
            )
        })?;
    let weeks = repos
        .training_plans
        .list_plan_weeks(&tenant_str, &user_str, &plan.id, false)
        .await?;
    let desired = desired_entries(params.user_id, &weeks, params.from);
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
    // did not). Those are adopted, never duplicated.
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
    let orphans: HashMap<String, CalendarEventRef> = observed
        .values()
        .filter_map(|event| {
            let key = event.external_id.clone()?;
            (key.starts_with(&plan_prefix) && !live_by_key.contains_key(&key))
                .then(|| (key, event.clone()))
        })
        .collect();
    let edit_slack = Duration::seconds(PROVIDER_EDIT_SLACK_SECONDS);
    let coach = plan.coach_slug.as_deref();

    let mut wanted: HashSet<String> = HashSet::new();
    for entry in &desired {
        let key = entry.session.external_id.clone();
        wanted.insert(key.clone());
        let hash = session_hash(&entry.session)?;
        let write = LedgerWrite {
            repos,
            params,
            coach,
            entry,
            hash: &hash,
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

/// One entry's write, with everything needed to record its outcome.
struct LedgerWrite<'a> {
    repos: &'a RepositoryRegistry,
    params: &'a PushPlanParams<'a>,
    coach: Option<&'a str>,
    entry: &'a DesiredEntry,
    hash: &'a str,
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
            coach_id: self.coach.map(str::to_owned),
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
        };
        self.repos
            .prescribed_workouts
            .upsert_prescribed_workout(&row)
            .await
    }
}
