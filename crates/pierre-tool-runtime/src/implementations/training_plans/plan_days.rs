// ABOUTME: The bounds a plan's days keep — the week's dates, the entries one date holds, each day's text, length, fuelling and steps
// ABOUTME: The agent's week save and the coach's day edits hold a day to the same checks; their messages name the session
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeSet;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::WorkoutStep;
use pierre_memory::training_plans::{session_positions, PlannedDay, MAX_SESSIONS_PER_DATE};

use super::{plan_date, WeekPayload, MAX_DURATION_MIN, MAX_TEXT_LEN};
use crate::implementations::calendar::{
    bounded, validate_step, TargetRule, MAX_SESSION_STEPS, MAX_SHORT_TEXT_LEN,
};
use crate::implementations::training_plan_vision::validate_day_template;

/// Fuelling ceilings. Nothing bounded what a plan could store, so a rate no
/// gut can absorb reached the athlete's calendar unchallenged; these are the
/// bound the save enforces.
const MAX_CARBS_G_PER_H: f32 = 150.0;
const MAX_FLUID_ML_PER_H: f32 = 1_500.0;
const MAX_SODIUM_MG_PER_H: f32 = 2_000.0;

/// Validate one week payload (dates, day dates inside the week, the entries
/// each date holds, each day's bounds through [`validate_day`]), and put its
/// days in date order. The sort is stable: the agent's order within a date
/// is kept, so the first entry on a date is its first session (carnet#1716).
/// A day's coach marks are dropped: the coach's own day saves stamp them,
/// never a payload (carnet#1714, carnet#1716).
pub(super) fn validate_week(week: &mut WeekPayload) -> AppResult<()> {
    let start = plan_date("week_start", &week.week_start)?;
    if week.days.is_empty() {
        return Err(AppError::invalid_input(format!(
            "week {} has no days",
            week.week_start
        )));
    }
    bounded("week.focus", &week.focus, MAX_TEXT_LEN)?;
    bounded(
        "week.adjustment_reason",
        &week.adjustment_reason,
        MAX_TEXT_LEN,
    )?;
    let positions = session_positions(&week.days);
    for (day, position) in week.days.iter().zip(positions) {
        let date = plan_date("day date", &day.date)?;
        let offset = (date - start).num_days();
        if !(0..7).contains(&offset) {
            return Err(AppError::invalid_input(format!(
                "{} falls outside week starting {}",
                session_label(&day.date, position),
                week.week_start
            )));
        }
    }
    // Canonical dates sort as they read, so the string order is the
    // calendar's; `sort_by` is stable.
    week.days.sort_by(|a, b| a.date.cmp(&b.date));
    validate_date_entries(&week.days)?;
    let positions = session_positions(&week.days);
    for (day, position) in week.days.iter_mut().zip(positions) {
        day.coach_original = None;
        day.coach_added = false;
        validate_day(day, position)?;
    }
    Ok(())
}

/// How a validation message names one session: its date and its 1-based
/// place among the date's entries, "day 2026-11-24 session 2" (carnet#1716).
pub(crate) fn session_label(date: &str, position: u32) -> String {
    format!("day {date} session {}", u64::from(position) + 1)
}

/// Hold every date of `days` to the entries one date may hold: at most
/// [`MAX_SESSIONS_PER_DATE`], and at most one rest entry beside its sessions
/// (carnet#1716). The agent's week save and the coach's day edits both hold a
/// week to it.
pub(crate) fn validate_date_entries(days: &[PlannedDay]) -> AppResult<()> {
    let mut rest_on: BTreeSet<&str> = BTreeSet::new();
    let positions = session_positions(days);
    for (day, position) in days.iter().zip(positions) {
        if !usize::try_from(position).is_ok_and(|at| at < MAX_SESSIONS_PER_DATE) {
            return Err(AppError::invalid_input(format!(
                "{} is past the {MAX_SESSIONS_PER_DATE} entries one date holds",
                session_label(&day.date, position)
            )));
        }
        if day.is_rest() && !rest_on.insert(day.date.as_str()) {
            return Err(AppError::invalid_input(format!(
                "{} is a second rest entry; a date holds at most one, beside its sessions",
                session_label(&day.date, position)
            )));
        }
    }
    Ok(())
}

/// Validate one day of a plan (workout, text bounds, duration, fuelling,
/// template reference, step bounds), and complete a structured day's
/// `duration_min` from its steps when the writer left it out — the stored day
/// never contradicts its own structure, and a stated duration that does is
/// refused. Its messages name the session at `position` on its date. The
/// agent's week save and the coach's hand-written day (carnet#1714) both hold
/// a day to these bounds.
pub(crate) fn validate_day(day: &mut PlannedDay, position: u32) -> AppResult<()> {
    let session = session_label(&day.date, position);
    if day.workout.trim().is_empty() {
        return Err(AppError::invalid_input(format!(
            "{session} has an empty workout"
        )));
    }
    bounded("day.sport", &day.sport, MAX_SHORT_TEXT_LEN)?;
    bounded("day.workout", &day.workout, MAX_TEXT_LEN)?;
    bounded("day.intensity", &day.intensity, MAX_SHORT_TEXT_LEN)?;
    if let Some(mins) = day.duration_min {
        if mins > MAX_DURATION_MIN {
            return Err(AppError::invalid_input(format!(
                "{session} duration_min {mins} exceeds {MAX_DURATION_MIN}"
            )));
        }
    }
    if let Some(fuel) = day.fueling.as_ref() {
        // Rates are per hour and cannot be negative, and each ceiling is
        // the schema's own. Sodium is checked like the rest even though it
        // is a loss estimate rather than a target — an implausible estimate
        // produces an implausible plan just as surely.
        for (field, value, max) in [
            ("carbs_g_per_h", fuel.carbs_g_per_h, MAX_CARBS_G_PER_H),
            ("fluid_ml_per_h", fuel.fluid_ml_per_h, MAX_FLUID_ML_PER_H),
        ] {
            if !value.is_finite() || !(0.0..=max).contains(&value) {
                return Err(AppError::invalid_input(format!(
                    "{session} fueling.{field} {value} is outside 0..={max}"
                )));
            }
        }
        if let Some(sodium) = fuel.sodium_mg_per_h {
            if !sodium.is_finite() || !(0.0..=MAX_SODIUM_MG_PER_H).contains(&sodium) {
                return Err(AppError::invalid_input(format!(
                    "{session} fueling.sodium_mg_per_h {sodium} is outside 0..={MAX_SODIUM_MG_PER_H}"
                )));
            }
        }
        if let Some(source) = fuel.carb_source.as_deref() {
            bounded("day.fueling.carb_source", source, MAX_SHORT_TEXT_LEN)?;
        }
        if day.is_rest() {
            return Err(AppError::invalid_input(format!(
                "{session} is a rest day and carries a fuelling protocol — there is no \
                 session to fuel"
            )));
        }
    }
    validate_day_template(day, &session)?;
    if day.steps.is_empty() {
        return Ok(());
    }
    if day.is_rest() {
        return Err(AppError::invalid_input(format!(
            "{session} is a rest day and carries steps — drop the steps or give the day a sport"
        )));
    }
    if day.steps.len() > MAX_SESSION_STEPS {
        return Err(AppError::invalid_input(format!(
            "{session} has {} steps; max {MAX_SESSION_STEPS}",
            day.steps.len()
        )));
    }
    for (index, step) in day.steps.iter().enumerate() {
        validate_step(
            &format!("{session} steps[{index}]"),
            step,
            TargetRule::Resolvable,
        )?;
    }
    let minutes = WorkoutStep::total_seconds(&day.steps).div_ceil(60);
    if minutes > u64::from(MAX_DURATION_MIN) {
        return Err(AppError::invalid_input(format!(
            "{session} steps total {minutes} minutes; max {MAX_DURATION_MIN}"
        )));
    }
    let minutes = u32::try_from(minutes)
        .map_err(|e| AppError::internal(format!("day duration does not fit in u32: {e}")))?;
    match day.duration_min {
        Some(stated) if stated != minutes => {
            return Err(AppError::invalid_input(format!(
                "{session} duration_min {stated} contradicts its steps, which total {minutes} \
                 minutes — omit duration_min or make it {minutes}"
            )));
        }
        Some(_) => {}
        None => day.duration_min = Some(minutes),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn payload(days: &Value) -> WeekPayload {
        serde_json::from_value(json!({ "week_start": "2026-11-23", "days": days })).unwrap()
    }

    fn entry(date: &str, sport: &str, workout: &str) -> Value {
        json!({ "date": date, "sport": sport, "workout": workout, "intensity": "Z2" })
    }

    fn message(result: AppResult<()>) -> String {
        result.expect_err("the week is refused").message
    }

    #[test]
    fn a_payload_day_never_sets_the_coachs_marks() {
        let mut payload: WeekPayload = serde_json::from_value(serde_json::json!({
            "week_start": "2026-11-16",
            "days": [{
                "date": "2026-11-18", "sport": "run", "workout": "Easy 50 min", "intensity": "Z2",
                "coach_original": {
                    "date": "2026-11-18", "sport": "run", "workout": "Easy 45 min", "intensity": "Z2"
                },
                "coach_added": true
            }]
        }))
        .unwrap();
        assert!(
            payload.days[0].coach_original.is_some() && payload.days[0].coach_added,
            "fixture: the payload names a snapshot and an added session"
        );
        validate_week(&mut payload).unwrap();
        assert!(
            payload.days[0].coach_original.is_none() && !payload.days[0].coach_added,
            "bug: the agent's save took a coach's mark from its payload"
        );
        assert_eq!(
            payload.days[0].workout, "Easy 50 min",
            "bug: the save keeps the day the payload wrote"
        );
    }

    #[test]
    fn a_date_holds_five_entries_and_refuses_a_sixth() {
        let date = "2026-11-24";
        let five: Vec<Value> = (1..=5)
            .map(|n| entry(date, "ride", &format!("Ride {n}")))
            .collect();
        let mut week = payload(&Value::Array(five.clone()));
        assert!(
            validate_week(&mut week).is_ok(),
            "bug: five entries on one date are saved"
        );

        let mut six = five;
        six.push(entry(date, "run", "Run 6"));
        assert_eq!(
            message(validate_week(&mut payload(&Value::Array(six)))),
            "day 2026-11-24 session 6 is past the 5 entries one date holds",
            "bug: a sixth entry on one date is refused, naming the session"
        );

        let mut week = payload(&json!([
            entry("2026-11-23", "ride", "a"),
            entry("2026-11-24", "ride", "b"),
            entry("2026-11-25", "ride", "c"),
            entry("2026-11-26", "ride", "d"),
            entry("2026-11-27", "ride", "e"),
            entry("2026-11-28", "ride", "f"),
            entry("2026-11-29", "ride", "g"),
            entry("2026-11-23", "strength", "h"),
        ]));
        assert!(
            validate_week(&mut week).is_ok(),
            "bug: a full week with a second session on Monday is saved (carnet#1716)"
        );
    }

    #[test]
    fn a_rest_entry_shares_its_date_with_sessions_and_a_second_rest_is_refused() {
        let date = "2026-11-24";
        let mut week = payload(&json!([
            entry(date, "rest", "Rest"),
            entry(date, "strength", "Optional: 20' light strength"),
        ]));
        assert!(
            validate_week(&mut week).is_ok(),
            "bug: a rest entry beside a session is saved"
        );

        let two_rests = json!([
            entry(date, "ride", "Spin"),
            entry(date, "rest", "Rest"),
            entry(date, "Rest", "Rest again"),
        ]);
        assert_eq!(
            message(validate_week(&mut payload(&two_rests))),
            "day 2026-11-24 session 3 is a second rest entry; a date holds at most one, beside \
             its sessions",
            "bug: a second rest entry on one date is refused, naming the session"
        );
    }

    #[test]
    fn the_save_puts_days_in_date_order_and_keeps_the_order_within_a_date() {
        let mut week = payload(&json!([
            entry("2026-11-25", "run", "Wed"),
            entry("2026-11-23", "ride", "Mon bike"),
            entry("2026-11-24", "run", "Tue"),
            entry("2026-11-23", "strength", "Mon strength"),
            entry("2026-11-23", "rest", "Mon rest"),
        ]));
        validate_week(&mut week).unwrap();
        let order: Vec<&str> = week.days.iter().map(|day| day.workout.as_str()).collect();
        assert_eq!(
            order,
            ["Mon bike", "Mon strength", "Mon rest", "Tue", "Wed"],
            "bug: the days sort by date and the agent's order within a date stands"
        );
    }

    #[test]
    fn a_validation_message_names_the_session_not_only_the_date() {
        let date = "2026-11-24";
        for (days, expected) in [
            (
                json!([entry(date, "ride", "Spin"), entry(date, "run", "  ")]),
                "day 2026-11-24 session 2 has an empty workout",
            ),
            (
                json!([entry(date, "ride", "Spin"),
                       { "date": date, "sport": "run", "workout": "Long", "duration_min": 1441 }]),
                "day 2026-11-24 session 2 duration_min 1441 exceeds 1440",
            ),
            (
                json!([entry(date, "ride", "Spin"), entry(date, "run", "Tempo"),
                       { "date": date, "sport": "rest", "workout": "Rest",
                         "steps": [{ "label": "x", "duration_seconds": 60, "target_zone": "Z1" }] }]),
                "day 2026-11-24 session 3 is a rest day and carries steps",
            ),
            (
                json!([entry("2026-12-07", "ride", "Spin")]),
                "day 2026-12-07 session 1 falls outside week starting 2026-11-23",
            ),
        ] {
            let refused = message(validate_week(&mut payload(&days)));
            assert!(
                refused.starts_with(expected),
                "bug: the message names the session: {days}: {refused}"
            );
        }
        assert_eq!(
            session_label(date, 0),
            "day 2026-11-24 session 1",
            "bug: a session is named 1-based"
        );
    }
}
