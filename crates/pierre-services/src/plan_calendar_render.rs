// ABOUTME: Render an athlete's plan days and week focuses into the calendar sessions a push writes — titles, notes, steps and keys
// ABOUTME: Pure and provider-agnostic: no database, no provider call; the push reconciles what this renders against the ledger
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Plan calendar rendering
//!
//! The rendering half of [`crate::plan_calendar_push`]: every non-rest
//! [`PlannedDay`] on or after the push date becomes one [`PlannedSession`]
//! keyed by [`CalendarKey::plan_day`], and a week with a focus one week note
//! keyed by [`CalendarKey::plan_week_note`]. The mapping is deterministic and
//! holds no model in the loop: the agent's prose goes out verbatim, and the
//! structure beside it is only what the agent stated — a day's `steps` when
//! it has them, else a single step when its `intensity` is inside
//! [`RelativeIntensity`]'s grammar, else a timed entry carrying the agent's
//! words, never a step the agent did not state.

use std::collections::HashMap;

use chrono::NaiveDate;
use pierre_core::models::{
    CalendarEventSource, CalendarKey, PlannedSession, PlannedSessionKind, RelativeIntensity,
    SportType, WorkoutStep,
};
use pierre_memory::training_plans::{parse_plan_date, PlanWeek, PlannedDay};
use uuid::Uuid;

use crate::plan_fuel_products::PlanFuelProducts;
use crate::plan_fueling::FuelingDisclosure;

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

/// The calendar title of a plan day: the first clause of the agent's
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
///
/// A day with `steps` is the structured case: the steps go out as they are
/// and the session lasts as long as they add up to, which is what lets the
/// provider build the workout and compute its planned load. A day without
/// them gets one structured step only when the agent stated an intensity
/// Dravr can express as a target; the sport name is the cue because a cue
/// drawn from the prose could carry a "2h" the provider's parser would read
/// as a duration.
///
/// `fueling` decides whether the note's fuel line carries the day's fuelling
/// rates or the statement that a clinician sets them, and names the fuel
/// products the day uses after them, as the plan card does.
#[must_use]
pub fn plan_day_session(
    user_id: Uuid,
    day: &PlannedDay,
    ordinal: usize,
    (disclosure, products): (&FuelingDisclosure, &PlanFuelProducts),
) -> Option<PlannedSession> {
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
    // The fuelling line rides the note because that is the field every
    // provider calendar renders. A protocol the agent prescribed and the
    // athlete cannot see on the day is the same as no protocol at all — and
    // for an athlete whose clinician sets the amounts, the line says so in
    // place of the rates.
    if let Some(line) = disclosure.day_line(day, products) {
        if !notes.is_empty() {
            notes.push('\n');
        }
        notes.push_str(&line);
    }
    let (duration_seconds, steps) = if day.steps.is_empty() {
        let duration_seconds = day
            .duration_min
            .filter(|minutes| *minutes > 0)
            .map(|minutes| minutes.saturating_mul(60));
        let steps = match (duration_seconds, RelativeIntensity::parse(intensity)) {
            (Some(seconds), Some(_)) => vec![WorkoutStep {
                label: sport.display_name().to_owned(),
                duration_seconds: seconds,
                distance_meters: None,
                target_zone: intensity.to_owned(),
                repeat: 1,
                repeat_group: None,
                note: None,
            }],
            _ => Vec::new(),
        };
        (duration_seconds, steps)
    } else {
        // The save bounds a day at 24 hours, so the total fits; saturating
        // is the honest answer for a row that somehow does not.
        let total = u32::try_from(WorkoutStep::total_seconds(&day.steps)).unwrap_or(u32::MAX);
        (Some(total), day.steps.clone())
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
/// past, and the past is not rewritten. `fueling` is the athlete's fuelling
/// disclosure and the names of the products the weeks reference, applied to
/// every day's note.
#[must_use]
pub fn desired_entries(
    user_id: Uuid,
    weeks: &[PlanWeek],
    from: NaiveDate,
    fueling: (&FuelingDisclosure, &PlanFuelProducts),
) -> Vec<DesiredEntry> {
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
            if let Some(session) = plan_day_session(user_id, day, this_ordinal, fueling) {
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
