// ABOUTME: Adjudicates a period figure against every reading the record vouches for, and folds the calls into one verdict
// ABOUTME: Totals and per-session, per-day and per-week averages, compared within tolerance or the figure's own rounding

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::SportType;
use pierre_memory::{ClaimStatus, EvidenceStrength, VerdictLayer};

use super::super::{
    contradicted, describe, unverifiable, AssertedFigure, RecordedActivity, Unit, MATCH_TOLERANCE,
};
use super::readings::{readings, Reading};
use super::{Per, PeriodHistory, Shape, Window};
use crate::verdict_engine::VerdictOutcome;

/// How one figure fared against the record.
pub(super) enum Call {
    /// It agrees with this reading.
    Agrees(String),
    /// Every reading is vouched for, none agrees, and the figure cannot be a
    /// part of one either; what the record holds.
    Differs(String),
    /// Every reading is vouched for and none agrees, but the figure has no
    /// total wording and falls short of a reading's total, so it may add up
    /// only some of its sessions; what the record holds.
    Short(String),
    /// No reading agrees, and at least one covers days the record lacks.
    Open,
}

/// How one figure fares against every reading the record vouches for.
pub(super) fn assess(
    figure: AssertedFigure,
    shape: Shape,
    windows: &[Window],
    history: &PeriodHistory,
    named: Option<&SportType>,
) -> Call {
    let mut open = false;
    let mut short = false;
    let mut held = Vec::new();
    for window in windows {
        if !history.covers(window) {
            open = true;
            continue;
        }
        for (index, reading) in readings(window, history, named).iter().enumerate() {
            if let Some(agreed) = agreement(figure, shape, window, reading) {
                return Call::Agrees(agreed);
            }
            short |= shape == Shape::Implicit && falls_short(figure, window, reading);
            // The first reading is the merged record for the sport the claim
            // names, or for all sports; the others only widen what can agree.
            if index == 0 {
                held.push(holding(figure, shape, window, reading));
            }
        }
    }
    if open || held.is_empty() {
        Call::Open
    } else if short {
        Call::Short(held.join("; "))
    } else {
        Call::Differs(held.join("; "))
    }
}

/// Whether a figure that agrees with nothing in `reading` falls short of its
/// total, and so may add up only some of its sessions.
///
/// Only an unworded figure is asked: one said to be the total or an average is
/// wrong below the record as much as above it. A figure above the total is
/// above each of its sessions too, since none is negative, so it is neither
/// the total, nor one session, nor any handful of them.
fn falls_short(figure: AssertedFigure, window: &Window, reading: &Reading<'_>) -> bool {
    aggregates(Shape::Total, window, &reading.sessions, figure.unit)
        .into_iter()
        .any(|total| figure.value < total)
}

/// The reading `figure` agrees with in this window, if any.
fn agreement(
    figure: AssertedFigure,
    shape: Shape,
    window: &Window,
    reading: &Reading<'_>,
) -> Option<String> {
    if aggregates(shape, window, &reading.sessions, figure.unit)
        .into_iter()
        .any(|value| agrees(figure, value))
    {
        return Some(format!("{} for {}", window.describe(), reading.label));
    }
    // With no aggregate wording the figure may be one session of the period.
    let single = shape == Shape::Implicit
        && reading
            .sessions
            .iter()
            .filter_map(|s| s.value_in(figure.unit))
            .any(|value| agrees(figure, value));
    single.then(|| format!("one {} session in {}", reading.label, window.describe()))
}

/// What the record holds for `figure`'s reading in this window, for the
/// explanation of a mismatch.
fn holding(figure: AssertedFigure, shape: Shape, window: &Window, reading: &Reading<'_>) -> String {
    let values = aggregates(shape, window, &reading.sessions, figure.unit);
    let shown = values.first().map_or_else(
        || "no session".to_owned(),
        |&value| describe(&[AssertedFigure { value, ..figure }]),
    );
    format!("{shown} in {} for {}", window.describe(), reading.label)
}

/// The aggregate values a reading yields: the total, or the average under each
/// way of counting its denominator. Empty when there is nothing to average.
fn aggregates(shape: Shape, window: &Window, picked: &[&RecordedActivity], unit: Unit) -> Vec<f64> {
    let values: Vec<f64> = picked.iter().filter_map(|s| s.value_in(unit)).collect();
    let total: f64 = values.iter().sum();
    match shape {
        Shape::Implicit | Shape::Total => vec![total],
        Shape::Average(Per::Session) => (!values.is_empty())
            .then(|| total / values.len() as f64)
            .into_iter()
            .collect(),
        Shape::Average(Per::Day) => vec![total / window.days().max(1) as f64],
        // Both ways a coach divides: by the window's length in weeks, and by
        // the calendar weeks it touches.
        Shape::Average(Per::Week) => vec![
            total / (window.days().max(1) as f64 / 7.0),
            total / window.weeks_touched().max(1) as f64,
        ],
        Shape::UnstatedAverage => Vec::new(),
    }
}

/// Whether a stated figure agrees with a computed one.
///
/// Within [`MATCH_TOLERANCE`] of the record, or within the rounding of the
/// stated figure: "6 h" for a 5 h 40 week is a coach rounding to the hour,
/// which 5% of 340 minutes would not allow.
fn agrees(figure: AssertedFigure, actual: f64) -> bool {
    let tolerance = (actual.abs() * MATCH_TOLERANCE).max(rounding_allowance(figure));
    (actual - figure.value).abs() <= tolerance
}

/// Half the step the stated figure was rounded to, in its canonical unit.
fn rounding_allowance(figure: AssertedFigure) -> f64 {
    const HOUR: f64 = 60.0;
    const HALF_HOUR: f64 = 30.0;
    let whole = |step: f64| (figure.value % step).abs() < f64::EPSILON;
    match figure.unit {
        Unit::Minutes if whole(HOUR) => HOUR / 2.0,
        Unit::Minutes if whole(HALF_HOUR) => HALF_HOUR / 2.0,
        _ if whole(1.0) => 0.5,
        _ => 0.05,
    }
}

/// One verdict from every figure's call.
///
/// Contradicted only when **every** figure differs. A sentence where one
/// figure agrees and another matches nothing usually carries a second
/// quantity this module does not model — a split, a zone time — and calling
/// it false would be a warning on a true sentence. An unworded figure below
/// the total is never one that differs: it may be some of the sessions.
pub(super) fn verdict(
    stated: &str,
    label: &str,
    calls: &[Call],
    windows: &[Window],
    history: &PeriodHistory,
) -> VerdictOutcome {
    let agreed: Vec<&str> = calls
        .iter()
        .filter_map(|call| match call {
            Call::Agrees(reading) => Some(reading.as_str()),
            _ => None,
        })
        .collect();
    let missed: Vec<&str> = calls
        .iter()
        .filter_map(|call| match call {
            Call::Differs(held) | Call::Short(held) => Some(held.as_str()),
            _ => None,
        })
        .collect();
    if agreed.len() == calls.len() {
        return supported(format!(
            "States {stated} for {label}, and it agrees with the record ({}).",
            agreed.join("; ")
        ));
    }
    if calls.iter().all(|call| matches!(call, Call::Differs(_))) {
        return contradicted(format!(
            "States {stated} for {label}, but the record holds {}.",
            missed.join("; ")
        ));
    }
    if calls.iter().any(|call| matches!(call, Call::Open)) {
        let spans: Vec<String> = windows.iter().map(Window::describe).collect();
        return unverifiable(&format!(
            "States {stated} for {label}, but {} reaches days the record does not hold: it is \
             complete from {} and synced through {}, so the claim cannot be settled.",
            spans.join(" or "),
            history.covered_from,
            history.synced_through
        ));
    }
    if agreed.is_empty() {
        return unverifiable(&format!(
            "States {stated} for {label}, and the record holds {}: with no word saying it is the \
             total, a figure below it that matches no session may add up only some of the \
             period's sessions (a club run, the last two runs), which this layer cannot pick \
             out, so it settles neither way.",
            missed.join("; ")
        ));
    }
    unverifiable(&format!(
        "States {stated} for {label}: part of it agrees with the record ({}) and part matches \
         nothing in that window ({}), which reads as a second quantity this layer does not \
         model, so it settles neither way.",
        agreed.join("; "),
        missed.join("; ")
    ))
}

/// The shape of a total or average the record agrees with.
fn supported(explanation: String) -> VerdictOutcome {
    VerdictOutcome {
        status: ClaimStatus::Supported,
        evidence_strength: EvidenceStrength::Strong,
        confidence: 0.9,
        layer_fired: VerdictLayer::AthleteData,
        explanation,
        evidence_refs: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn figure(value: f64, unit: Unit) -> AssertedFigure {
        AssertedFigure {
            value,
            unit,
            rate: false,
        }
    }

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    /// A run at six minutes a kilometre.
    fn run(date: NaiveDate, km: f64) -> RecordedActivity {
        RecordedActivity {
            date,
            sport: SportType::Run,
            name: String::new(),
            distance_km: Some(km),
            duration_min: km * 6.0,
            elevation_m: None,
            average_hr_bpm: None,
        }
    }

    /// Monday 08-31 to Wednesday 09-09: ten days touching two calendar weeks.
    fn block() -> Window {
        Window {
            reading: "the current block",
            start: day(8, 31),
            end: day(9, 9),
        }
    }

    fn near(got: &[f64], want: &[f64]) -> bool {
        got.len() == want.len() && got.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-9)
    }

    #[test]
    fn the_rounding_allowance_is_half_the_step_the_figure_was_rounded_to() {
        for (stated, allowance) in [
            (figure(360.0, Unit::Minutes), 30.0),
            (figure(330.0, Unit::Minutes), 15.0),
            (figure(335.0, Unit::Minutes), 0.5),
            (figure(42.0, Unit::Kilometres), 0.5),
            (figure(42.3, Unit::Kilometres), 0.05),
        ] {
            assert!(
                near(&[rounding_allowance(stated)], &[allowance]),
                "{stated:?} -> {allowance}"
            );
        }
    }

    #[test]
    fn a_figure_agrees_within_tolerance_or_within_its_own_rounding() {
        // "6 h" for a 5 h 40 week: 20 min off, past 5% of 340 min but inside
        // the half hour a coach rounding to the hour may be off by.
        assert!(agrees(figure(360.0, Unit::Minutes), 340.0));
        // "6 h" for 5 h 20: 40 min off, past both.
        assert!(!agrees(figure(360.0, Unit::Minutes), 320.0));
        // "335 min" for 340: inside 5%.
        assert!(agrees(figure(335.0, Unit::Minutes), 340.0));
        assert!(agrees(figure(21.0, Unit::Kilometres), 21.4));
        assert!(!agrees(figure(21.0, Unit::Kilometres), 30.0));
    }

    #[test]
    fn aggregates_divide_by_sessions_days_and_both_week_counts() {
        let sessions = [
            run(day(9, 1), 8.0),
            run(day(9, 3), 20.0),
            run(day(9, 7), 10.0),
            run(day(9, 8), 12.0),
        ];
        let picked: Vec<&RecordedActivity> = sessions.iter().collect();
        let window = block();
        let km = |shape| aggregates(shape, &window, &picked, Unit::Kilometres);
        assert!(near(&km(Shape::Total), &[50.0]));
        assert!(near(&km(Shape::Implicit), &[50.0]));
        assert!(near(&km(Shape::Average(Per::Session)), &[12.5]));
        assert!(near(&km(Shape::Average(Per::Day)), &[5.0]));
        // Ten days are 10/7 of a week (35 km a week) and touch two calendar
        // weeks (25 km a week).
        assert!(near(&km(Shape::Average(Per::Week)), &[35.0, 25.0]));
        let found = km(Shape::UnstatedAverage);
        assert!(found.is_empty(), "expected empty, got {found:?}");
    }

    #[test]
    fn a_per_session_average_counts_only_sessions_that_hold_the_unit() {
        let lift = RecordedActivity {
            sport: SportType::StrengthTraining,
            distance_km: None,
            duration_min: 45.0,
            ..run(day(9, 8), 0.0)
        };
        let sessions = [run(day(9, 7), 10.0), run(day(9, 8), 12.0), lift];
        let picked: Vec<&RecordedActivity> = sessions.iter().collect();
        let window = block();
        let per_session = Shape::Average(Per::Session);
        // The lift has no distance: 22 km over the two runs.
        assert!(near(
            &aggregates(per_session, &window, &picked, Unit::Kilometres),
            &[11.0]
        ));
        // 60 + 72 + 45 minutes over all three.
        assert!(near(
            &aggregates(per_session, &window, &picked, Unit::Minutes),
            &[59.0]
        ));
        let found = aggregates(per_session, &window, &[], Unit::Kilometres);
        assert!(found.is_empty(), "expected empty, got {found:?}");
    }

    #[test]
    fn only_a_figure_below_the_total_falls_short() {
        let sessions = [run(day(9, 7), 8.0), run(day(9, 8), 10.0)];
        let reading = Reading {
            label: "all sports".to_owned(),
            sessions: sessions.iter().collect(),
        };
        let window = block();
        let short = |km| falls_short(figure(km, Unit::Kilometres), &window, &reading);
        // 18 km on record: 10 km may be one of the runs, 30 km is none of them.
        assert!(short(10.0));
        assert!(!short(30.0));
    }
}
