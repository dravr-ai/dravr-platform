// ABOUTME: A coach's own edits of a session of their draft — written by hand or made a rest day, added, removed, or put back
// ABOUTME: Each names the session as the client saw it; an edit keeps coach_original, an added session coach_added, and restore undoes either

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The coach edits a session in place (carnet#1714, carnet#1716)
//!
//! The editor opens on a session of the day. The coach edits it — sport,
//! name, duration, intensity, steps — or makes it a rest entry, and
//! [`save_draft_hand_day`] saves it into their draft, never to the athlete
//! before Publish. The session passes the bounds the agent's own plan save
//! holds a day to ([`validate_day`]); it names no family, so no envelope
//! check measures it.
//!
//! A date holds several sessions. The coach adds one at the end of the
//! date's entries ([`add_draft_session`]), built from a family or written by
//! hand, and removes one of several ([`remove_draft_day`]); the last entry on
//! a date is made a rest day instead, never removed.
//!
//! Every edit of the coach's keeps the session as it stood before their
//! first edit in this draft (`coach_original`), and a session they added is
//! marked so (`coach_added`); [`restore_draft_day`] puts the first back and
//! removes the second. The marks are the coach's editor's alone: the agent's
//! save never takes them from a payload, no agent read serves them, and
//! publishing drops them.

use std::future::Future;
use std::sync::Arc;

use dravr_tronc::dates::CivilDate;
use pierre_contremaitre::messaging_strings::KEY_PLAN_REST;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::periodization::serde_num::whole_u32_opt;
use pierre_core::models::{RelativeIntensity, WorkoutStep};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::{PlanWeek, PlannedDay, MAX_SESSIONS_PER_DATE};
use pierre_providers::ai_scope;
use pierre_services::athlete_anchors::stored_athlete_anchors;
use pierre_services::coach_plan_draft::CoachDraftTarget;
use pierre_services::provider_notice;
use serde::Deserialize;
use serde_json::json;
use tracing::info;

use super::sessions::{
    entries_on, expected_index, placed, stamp_edit, without, SessionAt, INVALID_SESSION,
};
use super::{
    build_day, refused, save_days_into_week, week_holding, writer_catalogue, DraftDayOutcome,
    EditorThread, EditorWords, SessionBuild, WeekFrom,
};
use crate::implementations::training_plans::validate_day;
use crate::runtime::ToolRuntime;

/// The sport a rest day is written with, as the agent writes one
/// ([`PlannedDay::is_rest`]).
const REST_SPORT: &str = "rest";

/// The `details.reason` of a hand-written step whose target the calendar's
/// intensity grammar does not read; `details.step` (1-based) and
/// `details.target` name it, so the coach's editor says which step in the
/// coach's language rather than the server's sentence (carnet#1727).
const UNRESOLVABLE_TARGET: &str = "unresolvable_target";

/// The first step, 1-based, whose target the calendar's intensity grammar
/// does not read, with that target. An empty target is the bounds check's to
/// refuse as a missing one.
fn unresolvable_step(steps: &[WorkoutStep]) -> Option<(usize, &str)> {
    steps.iter().enumerate().find_map(|(at, step)| {
        let target = step.target_zone.trim();
        (!target.is_empty() && RelativeIntensity::parse(target).is_none())
            .then_some((at + 1, target))
    })
}

/// The refusal of a step whose target the calendar cannot resolve, naming
/// the step and its target for the client to word.
fn unresolvable_target(step: usize, target: &str) -> AppError {
    let mut error = AppError::invalid_input(format!(
        "step {step}'s target '{target}' is not one the calendar can resolve: use a zone, \
         a percent of a threshold or an RPE band"
    ));
    error.details = Some(Box::new(
        json!({ "reason": UNRESOLVABLE_TARGET, "step": step, "target": target }),
    ));
    error
}

/// A day the coach writes by hand: a session, or a rest day.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HandDay {
    /// `true` for a rest day; every other field is then left unread.
    pub rest: bool,
    /// The day's sport, in the plan's own words (`run`, `gravel`, …).
    #[serde(default)]
    pub sport: String,
    /// The session's name.
    #[serde(default)]
    pub workout: String,
    /// Its length in minutes. With steps, left out it is their total, and
    /// stated it must be.
    #[serde(default, deserialize_with = "whole_u32_opt")]
    pub duration_min: Option<u32>,
    /// Its intensity, relative to the athlete's thresholds.
    #[serde(default)]
    pub intensity: String,
    /// Its structure in order, each step as the plan card shows one.
    #[serde(default)]
    pub steps: Vec<WorkoutStep>,
}

/// The session a coach adds to a date: exactly one of a family build or a
/// day written by hand.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddedSession {
    /// A session built from a workout family.
    Build(SessionBuild),
    /// A session written by hand, or a rest entry.
    Day(HandDay),
}

/// What an edit makes of a week's days, or the outcome that leaves nothing
/// written.
type Edited = Result<Vec<PlannedDay>, DraftDayOutcome>;

/// The rest-day session's name in the coach's words.
///
/// # Errors
///
/// An internal error when the server strings name none.
fn rest_label(words: EditorWords<'_>) -> AppResult<String> {
    let label = words.strings.get(KEY_PLAN_REST, words.locale);
    if label.trim().is_empty() {
        return Err(AppError::internal(
            "the server strings name no rest-day session",
        ));
    }
    Ok(label)
}

/// The day `request` writes on `date` at `position` among its entries, over
/// `replaced`, a rest day's session named `rest_label`: as the coach wrote
/// it, naming no template and carrying no envelope, the replaced day's
/// fuelling kept while the day stays a session. The coach's marks are the
/// caller's to stamp.
///
/// # Errors
///
/// Invalid input with `details.reason` `unresolvable_target` for a step
/// whose target the calendar does not read, else `invalid_session` for a day
/// past the bounds the agent's plan save holds a day to, naming the session.
fn hand_day(
    request: &HandDay,
    date: CivilDate,
    position: u32,
    replaced: Option<&PlannedDay>,
    rest_label: &str,
) -> AppResult<PlannedDay> {
    let (sport, workout, duration_min, intensity, steps) = if request.rest {
        (
            REST_SPORT.to_owned(),
            rest_label.to_owned(),
            None,
            String::new(),
            Vec::new(),
        )
    } else {
        (
            request.sport.clone(),
            request.workout.clone(),
            request.duration_min,
            request.intensity.clone(),
            request.steps.clone(),
        )
    };
    let mut day = PlannedDay {
        date: date.to_string(),
        sport,
        workout,
        duration_min,
        intensity,
        steps,
        fueling: replaced.and_then(|day| day.fueling.clone()),
        template_slug: None,
        template_params: None,
        template_source: None,
        template_preset: None,
        envelope: None,
        coach_original: None,
        coach_added: false,
    };
    if day.is_rest() {
        day.fueling = None;
    } else if let Some((step, target)) = unresolvable_step(&day.steps) {
        return Err(unresolvable_target(step, target));
    }
    validate_day(&mut day, position).map_err(|error| refused(INVALID_SESSION, error.message))?;
    Ok(day)
}

/// `days` with the session `at` names on `date` written by hand as
/// `request`, the coach's marks stamped.
///
/// # Errors
///
/// The refusals of [`hand_day`] and [`placed`].
fn hand_edited(
    days: &[PlannedDay],
    date: CivilDate,
    at: &SessionAt,
    request: &HandDay,
    rest_label: &str,
) -> AppResult<Edited> {
    let index = match expected_index(days, &date.to_string(), at) {
        Ok(index) => index,
        Err(outcome) => return Ok(Err(outcome)),
    };
    let replaced = days.get(index);
    let mut day = hand_day(request, date, at.position, replaced, rest_label)?;
    stamp_edit(&mut day, replaced);
    placed(days, Some(index), day).map(Ok)
}

/// `days` with the session `at` names on `date` put back: a session the
/// coach added is removed, an edited one is the day its snapshot holds.
/// `NothingToRestore` for a session the coach neither edited nor added.
///
/// # Errors
///
/// The refusal of [`placed`] when the day put back would be a second rest
/// entry on its date.
fn restored(days: &[PlannedDay], date: &str, at: &SessionAt) -> AppResult<Edited> {
    let index = match expected_index(days, date, at) {
        Ok(index) => index,
        Err(outcome) => return Ok(Err(outcome)),
    };
    let Some(session) = days.get(index) else {
        return Ok(Err(DraftDayOutcome::SessionMoved));
    };
    if session.coach_added {
        return Ok(Ok(without(days, index)));
    }
    session.coach_original.as_deref().map_or_else(
        || Ok(Err(DraftDayOutcome::NothingToRestore)),
        |original| placed(days, Some(index), original.clone()).map(Ok),
    )
}

/// `days` without the session `at` names on `date`; `LastSession` when it is
/// the date's only entry.
fn removed(days: &[PlannedDay], date: &str, at: &SessionAt) -> Edited {
    let index = expected_index(days, date, at)?;
    if entries_on(days, date) <= 1 {
        return Err(DraftDayOutcome::LastSession);
    }
    Ok(without(days, index))
}

/// The position a session added to `date` takes: after every entry the date
/// holds, or `DateFull` when it holds as many as a date holds.
fn next_position(days: &[PlannedDay], date: &str) -> Result<u32, DraftDayOutcome> {
    let held = entries_on(days, date);
    if held >= MAX_SESSIONS_PER_DATE {
        return Err(DraftDayOutcome::DateFull);
    }
    u32::try_from(held).map_err(|_| DraftDayOutcome::DateFull)
}

/// `days` with `day` added after its date's entries, marked as a session the
/// coach added, with no snapshot.
///
/// # Errors
///
/// The refusal of [`placed`] for a second rest entry on the date.
fn added(days: &[PlannedDay], mut day: PlannedDay) -> AppResult<Vec<PlannedDay>> {
    day.coach_added = true;
    day.coach_original = None;
    placed(days, None, day)
}

/// Run `write` on the athlete's record: tracked so the draft is stamped from
/// what it read (carnet#769), under the athlete's AI consents (carnet#1419).
async fn as_athletes_write(
    repos: &RepositoryRegistry,
    target: &CoachDraftTarget,
    write: impl Future<Output = AppResult<DraftDayOutcome>> + Send,
) -> AppResult<DraftDayOutcome> {
    let tracked = Box::pin(ai_scope::tracking(ai_scope::Provenance::new(), write));
    provider_notice::under_subject_ai_consent(
        repos,
        target.tenant().as_uuid(),
        target.athlete(),
        tracked,
    )
    .await
}

/// What a saved edit logs beside the athlete, the coach and the draft: never
/// a session's content.
struct EditLog {
    /// Which edit: `hand`, `restore`, `remove` or `add`.
    edit: &'static str,
    /// The date edited.
    date: CivilDate,
    /// The session's position on it.
    position: u32,
}

/// Save `edited` as `week` of the coach's draft, or answer the outcome that
/// leaves nothing written.
///
/// # Errors
///
/// The repository error of the save.
async fn save_edit(
    repos: &RepositoryRegistry,
    thread: &EditorThread,
    week: &PlanWeek,
    edited: Edited,
    log: EditLog,
) -> AppResult<DraftDayOutcome> {
    let days = match edited {
        Ok(days) => days,
        Err(outcome) => return Ok(outcome),
    };
    let saved = save_days_into_week(repos, thread, week, &days).await?;
    info!(
        athlete_user_id = %thread.target.athlete(),
        coach_user_id = %thread.target.coach(),
        draft_id = %saved.draft.plan.id,
        opened = saved.opened,
        date = %log.date,
        position = log.position,
        edit = log.edit,
        "coach edited a session of their draft of a coached athlete's plan"
    );
    Ok(DraftDayOutcome::Saved)
}

/// Save the session the coach wrote by hand over the session `at` names on
/// `date` of their draft of the athlete's plan, a rest entry's session named
/// in the coach's words.
///
/// The week holding `date` is read from the draft — over the live plan when
/// the coach holds none, which the save then opens it from — and that
/// session replaced, the week's other days as they stand. The athlete sees
/// nothing until the coach publishes.
///
/// # Errors
///
/// The refusal of a step target the calendar does not read
/// (`unresolvable_target`), of a day past the plan's day bounds or a second
/// rest entry on the date (`invalid_session`), `UnavailableOverTransport` for
/// a plan an external caller may not read, and the repository errors of the
/// reads and the save.
pub async fn save_draft_hand_day(
    runtime: &Arc<dyn ToolRuntime>,
    thread: &EditorThread,
    date: CivilDate,
    at: &SessionAt,
    request: &HandDay,
    words: EditorWords<'_>,
) -> AppResult<DraftDayOutcome> {
    let rest_label = rest_label(words)?;
    let repos = runtime.repos();
    let target = &thread.target;
    let write = async {
        ai_scope::note_member_read(target.athlete());
        let week = match week_holding(repos, target, date, WeekFrom::DraftOrLive).await? {
            Ok(week) => week,
            Err(outcome) => return Ok(outcome),
        };
        let edited = hand_edited(&week.days, date, at, request, &rest_label)?;
        let log = EditLog {
            edit: "hand",
            date,
            position: at.position,
        };
        save_edit(repos, thread, &week, edited, log).await
    };
    as_athletes_write(repos, target, write).await
}

/// Put the session `at` names on `date` of the coach's draft back.
///
/// A session the coach added is removed, an edited one is put back as it
/// stood before their first edit in this draft, and the week's other days
/// stay as they stand. Restore never opens a draft.
///
/// # Errors
///
/// The refusal of a second rest entry on the date (`invalid_session`),
/// `UnavailableOverTransport` for a plan an external caller may not read, and
/// the repository errors of the reads and the save.
pub async fn restore_draft_day(
    runtime: &Arc<dyn ToolRuntime>,
    thread: &EditorThread,
    date: CivilDate,
    at: &SessionAt,
) -> AppResult<DraftDayOutcome> {
    let repos = runtime.repos();
    let target = &thread.target;
    let write = async {
        ai_scope::note_member_read(target.athlete());
        let week = match week_holding(repos, target, date, WeekFrom::Draft).await? {
            Ok(week) => week,
            Err(outcome) => return Ok(outcome),
        };
        let edited = restored(&week.days, &date.to_string(), at)?;
        let log = EditLog {
            edit: "restore",
            date,
            position: at.position,
        };
        save_edit(repos, thread, &week, edited, log).await
    };
    as_athletes_write(repos, target, write).await
}

/// Remove the session `at` names from `date` of the coach's draft, opened
/// from the live plan when the coach holds none. The last entry on a date is
/// never removed: the coach makes it a rest day.
///
/// # Errors
///
/// `UnavailableOverTransport` for a plan an external caller may not read,
/// and the repository errors of the reads and the save.
pub async fn remove_draft_day(
    runtime: &Arc<dyn ToolRuntime>,
    thread: &EditorThread,
    date: CivilDate,
    at: &SessionAt,
) -> AppResult<DraftDayOutcome> {
    let repos = runtime.repos();
    let target = &thread.target;
    let write = async {
        ai_scope::note_member_read(target.athlete());
        let week = match week_holding(repos, target, date, WeekFrom::DraftOrLive).await? {
            Ok(week) => week,
            Err(outcome) => return Ok(outcome),
        };
        let edited = removed(&week.days, &date.to_string(), at);
        let log = EditLog {
            edit: "remove",
            date,
            position: at.position,
        };
        save_edit(repos, thread, &week, edited, log).await
    };
    as_athletes_write(repos, target, write).await
}

/// Add `session` after the entries on `date` of the coach's draft.
///
/// The draft opens from the live plan when the coach holds none, and the
/// session is marked as one the coach added. The date need hold no entry
/// yet, but a week of the plan must hold it.
///
/// # Errors
///
/// The refusals of [`build_day`] for a family build, of a step target the
/// calendar does not read (`unresolvable_target`), of a day past the plan's
/// day bounds or a second rest entry on the date (`invalid_session`),
/// `UnavailableOverTransport` for a plan an external caller may not read, and
/// the repository errors of the reads and the save.
pub async fn add_draft_session(
    runtime: &Arc<dyn ToolRuntime>,
    thread: &EditorThread,
    date: CivilDate,
    session: &AddedSession,
    words: EditorWords<'_>,
) -> AppResult<DraftDayOutcome> {
    let repos = runtime.repos();
    let target = &thread.target;
    let write = async {
        ai_scope::note_member_read(target.athlete());
        let week = match week_holding(repos, target, date, WeekFrom::DraftOrLive).await? {
            Ok(week) => week,
            Err(outcome) => return Ok(outcome),
        };
        let position = match next_position(&week.days, &date.to_string()) {
            Ok(position) => position,
            Err(outcome) => return Ok(outcome),
        };
        let day = match session {
            AddedSession::Build(build) => {
                let catalogue = writer_catalogue(runtime, &thread.writer).await?;
                let anchors =
                    stored_athlete_anchors(repos, target.tenant(), target.athlete()).await?;
                let label = build.sport.as_str().to_owned();
                build_day(&catalogue, build, date, position, label, anchors.as_ref())?.day
            }
            AddedSession::Day(request) => {
                hand_day(request, date, position, None, &rest_label(words)?)?
            }
        };
        let edited = added(&week.days, day)?;
        let log = EditLog {
            edit: "add",
            date,
            position,
        };
        save_edit(repos, thread, &week, Ok(edited), log).await
    };
    as_athletes_write(repos, target, write).await
}

#[cfg(test)]
mod tests {
    use super::super::sessions::SessionExpect;
    use super::*;
    use pierre_memory::training_plans::{parse_plan_date, TemplateSource};
    use serde_json::{json, Value};

    const DATE: &str = "2026-11-18";

    fn date() -> CivilDate {
        parse_plan_date(DATE).unwrap()
    }

    fn stored(value: Value) -> PlannedDay {
        serde_json::from_value(value).unwrap()
    }

    fn request(value: Value) -> HandDay {
        serde_json::from_value(value).unwrap()
    }

    /// A family day with its fuelling, as the save stores one.
    fn family_day() -> PlannedDay {
        stored(json!({
            "date": DATE, "sport": "ride", "workout": "VO2max — 30/15", "intensity": "100-110% MAP",
            "duration_min": 61, "template_slug": "vo2max_short", "template_preset": "30_15",
            "template_params": { "sets": 3 }, "template_source": "catalogue",
            "envelope": { "generated": true },
            "fueling": { "carbs_g_per_h": 60.0, "fluid_ml_per_h": 500.0 }
        }))
    }

    fn easy_fifty() -> HandDay {
        request(json!({
            "rest": false, "sport": "run", "workout": "Easy 50 min", "intensity": "Z2",
            "steps": [
                { "label": "Warm-up", "duration_seconds": 600, "target_zone": "Z1" },
                { "label": "Strides", "duration_seconds": 20, "target_zone": "Z5", "repeat": 6 },
                { "label": "Steady", "duration_seconds": 2280, "target_zone": "Z2" }
            ]
        }))
    }

    fn at(position: u32, sport: &str, workout: &str) -> SessionAt {
        SessionAt {
            position,
            expect: SessionExpect {
                sport: sport.to_owned(),
                workout: workout.to_owned(),
            },
        }
    }

    fn workouts(days: &[PlannedDay]) -> Vec<&str> {
        days.iter().map(|day| day.workout.as_str()).collect()
    }

    /// The agent's two sessions on DATE beside Tuesday's run.
    fn brick_week() -> Vec<PlannedDay> {
        vec![
            stored(json!({ "date": "2026-11-17", "sport": "run", "workout": "Easy" })),
            family_day(),
            stored(json!({ "date": DATE, "sport": "run", "workout": "Brick run" })),
        ]
    }

    fn reason(error: &AppError) -> &str {
        error
            .details
            .as_ref()
            .and_then(|details| details["reason"].as_str())
            .unwrap_or_default()
    }

    #[test]
    fn a_hand_day_names_no_template_and_keeps_the_replaced_days_fuelling() {
        let replaced = family_day();
        assert_eq!(
            replaced.template_source,
            Some(TemplateSource::Catalogue),
            "fixture: the replaced day is a family day"
        );
        let day = hand_day(&easy_fifty(), date(), 0, Some(&replaced), "rest").unwrap();
        assert_eq!(day.date, DATE, "bug: the day is written on its date");
        assert_eq!(day.sport, "run", "bug: the day takes the coach's sport");
        assert_eq!(
            day.workout, "Easy 50 min",
            "bug: the day takes the coach's name"
        );
        assert_eq!(
            day.intensity, "Z2",
            "bug: the day takes the coach's intensity"
        );
        assert_eq!(
            day.steps.len(),
            3,
            "bug: the day keeps the coach's steps: {:?}",
            day.steps
        );
        assert_eq!(
            day.duration_min,
            Some(50),
            "bug: a day with steps and no stated length takes their total"
        );
        assert!(
            day.template_slug.is_none()
                && day.template_params.is_none()
                && day.template_source.is_none()
                && day.template_preset.is_none()
                && day.envelope.is_none(),
            "bug: a hand-written day names no family and carries no envelope: {day:?}"
        );
        assert_eq!(
            day.fueling, replaced.fueling,
            "bug: a day that stays a session keeps the replaced day's fuelling"
        );

        let rest = hand_day(
            &request(json!({ "rest": true })),
            date(),
            0,
            Some(&replaced),
            "repos",
        )
        .unwrap();
        assert!(rest.is_rest(), "bug: a rest day is written as a rest day");
        assert_eq!(
            rest.workout, "repos",
            "bug: a rest day's session is named in the coach's words"
        );
        assert!(
            rest.fueling.is_none() && rest.steps.is_empty() && rest.duration_min.is_none(),
            "bug: a rest day carries no fuelling, steps or length: {rest:?}"
        );
        assert!(
            rest.template_slug.is_none() && rest.envelope.is_none(),
            "bug: a rest day names no family: {rest:?}"
        );
    }

    #[test]
    fn a_hand_day_past_the_plans_day_bounds_is_refused() {
        let step = |target: &str, repeat: u32| json!({ "label": "Work", "duration_seconds": 60, "target_zone": target, "repeat": repeat });
        let many: Vec<Value> = (0..51).map(|_| step("Z2", 1)).collect();
        for body in [
            json!({ "rest": false, "sport": "run", "workout": "  ", "intensity": "Z2" }),
            json!({ "rest": false, "sport": "run", "workout": "Long", "duration_min": 1441 }),
            json!({ "rest": false, "sport": "run", "workout": "Reps", "steps": [step("Z2", 101)] }),
            json!({ "rest": false, "sport": "run", "workout": "Reps", "steps": [step(" ", 1)] }),
            json!({ "rest": false, "sport": "run", "workout": "Reps", "steps": many }),
            json!({ "rest": false, "sport": "run", "workout": "Reps", "duration_min": 30,
                    "steps": [step("Z2", 1)] }),
            json!({ "rest": false, "sport": "rest", "workout": "Off", "steps": [step("Z2", 1)] }),
        ] {
            let refused = hand_day(&request(body.clone()), date(), 0, None, "rest").unwrap_err();
            assert_eq!(
                reason(&refused),
                INVALID_SESSION,
                "bug: a day past the plan's day bounds is refused by its reason: {body}: {refused}"
            );
        }
    }

    #[test]
    fn a_step_target_the_calendar_cannot_read_is_refused_naming_its_step() {
        let step = |target: &str| json!({ "label": "Work", "duration_seconds": 60, "target_zone": target });
        let body = json!({
            "rest": false, "sport": "run", "workout": "Reps",
            "steps": [step("Z2"), step("RPE 7"), step("seuil")],
        });
        let refused = hand_day(&request(body), date(), 0, None, "rest").unwrap_err();
        assert_eq!(
            refused.details.as_deref().cloned(),
            Some(json!({ "reason": UNRESOLVABLE_TARGET, "step": 3, "target": "seuil" })),
            "bug: an unreadable target names its 1-based step and the target as written: {refused}"
        );
        let readable = json!({
            "rest": false, "sport": "run", "workout": "Reps",
            "steps": [step("Z2"), step("88-93% FTP"), step("sweet spot")],
        });
        assert!(
            hand_day(&request(readable), date(), 0, None, "rest").is_ok(),
            "bug: every target the grammar reads is accepted"
        );
    }

    #[test]
    fn a_hand_edit_writes_its_session_and_keeps_the_first_edits_snapshot() {
        let days = brick_week();
        let edited = hand_edited(
            &days,
            date(),
            &at(1, "run", "Brick run"),
            &easy_fifty(),
            "rest",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            workouts(&edited),
            ["Easy", "VO2max — 30/15", "Easy 50 min"],
            "bug: the hand edit replaces only the session it names"
        );
        assert_eq!(
            edited[2]
                .coach_original
                .as_ref()
                .map(|day| day.workout.as_str()),
            Some("Brick run"),
            "bug: the edit keeps the session it replaced"
        );

        let again = hand_edited(
            &edited,
            date(),
            &at(1, "run", "Easy 50 min"),
            &request(json!({ "rest": true })),
            "rest",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            again[2]
                .coach_original
                .as_ref()
                .map(|day| day.workout.as_str()),
            Some("Brick run"),
            "bug: a second edit keeps the first edit's snapshot"
        );
        assert!(
            again[2]
                .coach_original
                .as_ref()
                .is_some_and(|snapshot| snapshot.coach_original.is_none()),
            "bug: a snapshot never carries one of its own"
        );
    }

    #[test]
    fn an_edit_of_a_session_that_moved_writes_nothing() {
        let days = brick_week();
        for stale in [
            at(1, "run", "Brick run 40"),
            at(0, "run", "Brick run"),
            at(2, "run", "Brick run"),
        ] {
            assert_eq!(
                hand_edited(&days, date(), &stale, &easy_fifty(), "rest").unwrap(),
                Err(DraftDayOutcome::SessionMoved),
                "bug: a hand edit of a session that moved writes nothing: {stale:?}"
            );
            assert_eq!(
                restored(&days, DATE, &stale).unwrap(),
                Err(DraftDayOutcome::SessionMoved),
                "bug: a restore of a session that moved writes nothing: {stale:?}"
            );
            assert_eq!(
                removed(&days, DATE, &stale),
                Err(DraftDayOutcome::SessionMoved),
                "bug: removing a session that moved writes nothing: {stale:?}"
            );
        }
    }

    #[test]
    fn an_added_session_takes_the_dates_next_position_and_restoring_it_removes_it() {
        let days = brick_week();
        assert_eq!(
            next_position(&days, DATE),
            Ok(2),
            "bug: an added session goes after the date's two entries"
        );
        let core = hand_day(
            &request(json!({ "rest": false, "sport": "strength", "workout": "Core 20 min" })),
            date(),
            2,
            None,
            "rest",
        )
        .unwrap();
        let with_core = added(&days, core).unwrap();
        assert_eq!(
            workouts(&with_core),
            ["Easy", "VO2max — 30/15", "Brick run", "Core 20 min"],
            "bug: the added session is the date's last entry"
        );
        assert!(
            with_core[3].coach_added && with_core[3].coach_original.is_none(),
            "bug: an added session is marked added, with no snapshot"
        );

        let back = restored(&with_core, DATE, &at(2, "strength", "Core 20 min"))
            .unwrap()
            .unwrap();
        assert_eq!(
            back, days,
            "bug: restoring an added session removes it and leaves the week as it was"
        );
        assert_eq!(
            restored(&days, DATE, &at(0, "ride", "VO2max — 30/15")).unwrap(),
            Err(DraftDayOutcome::NothingToRestore),
            "bug: a session the coach neither edited nor added has nothing to restore"
        );

        let full: Vec<PlannedDay> = (0..MAX_SESSIONS_PER_DATE)
            .map(|n| stored(json!({ "date": DATE, "sport": "ride", "workout": n.to_string() })))
            .collect();
        assert_eq!(
            next_position(&full, DATE),
            Err(DraftDayOutcome::DateFull),
            "bug: a date holding five entries takes no sixth"
        );
        let rest = hand_day(&request(json!({ "rest": true })), date(), 2, None, "rest").unwrap();
        let rested = added(&days, rest.clone()).unwrap();
        let refused = added(&rested, rest).unwrap_err();
        assert_eq!(
            reason(&refused),
            INVALID_SESSION,
            "bug: a second rest entry on a date is refused as an invalid session"
        );
    }

    #[test]
    fn restoring_an_edited_session_puts_back_its_snapshot() {
        let days = brick_week();
        let edited = hand_edited(
            &days,
            date(),
            &at(0, "ride", "VO2max — 30/15"),
            &easy_fifty(),
            "rest",
        )
        .unwrap()
        .unwrap();
        let back = restored(&edited, DATE, &at(0, "run", "Easy 50 min"))
            .unwrap()
            .unwrap();
        assert_eq!(
            back, days,
            "bug: restoring puts back the session as it stood, and leaves the others"
        );
    }

    #[test]
    fn one_of_several_sessions_is_removed_and_the_last_one_is_not() {
        let days = brick_week();
        let one_left = removed(&days, DATE, &at(0, "ride", "VO2max — 30/15")).unwrap();
        assert_eq!(
            workouts(&one_left),
            ["Easy", "Brick run"],
            "bug: removing takes only the session it names"
        );
        assert_eq!(
            removed(&one_left, DATE, &at(0, "run", "Brick run")),
            Err(DraftDayOutcome::LastSession),
            "bug: the last entry on a date is never removed"
        );
    }
}
