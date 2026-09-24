// ABOUTME: Unit tests for the plan ramp check — opening week against recent load
// ABOUTME: Pins the threshold boundary and that every unmeasurable case is reported, never passed

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used)]

use pierre_core::models::LoadSnapshot;
use pierre_services::ramp_check::{
    assess_ramp, planned_week_minutes, RampUnmeasurable, RampVerdict,
};

fn baseline(weekly_hours: f64) -> LoadSnapshot {
    LoadSnapshot {
        weekly_hours,
        sessions_per_week: 4.0,
        longest_session_min: 120,
        weeks: 6,
        sport_families: 1,
    }
}

#[test]
fn a_plan_opening_far_above_recent_load_warns() {
    // 12h planned against a 7h baseline is +71% — the case the rail exists
    // for, and the one a week-over-week check never sees.
    let week = vec![Some(180), Some(180), Some(180), Some(180)];
    let verdict = assess_ramp(&week, Some(&baseline(7.0)));
    let increase = match &verdict {
        RampVerdict::Exceeded { increase, .. } => *increase,
        _ => 0.0,
    };
    assert!(
        increase > 0.70,
        "a 12h opening week on a 7h baseline must warn, got {verdict:?}"
    );
}

#[test]
fn a_plan_opening_near_recent_load_does_not_warn() {
    let week = vec![Some(120), Some(120), Some(90), Some(90)];
    assert!(matches!(
        assess_ramp(&week, Some(&baseline(7.0))),
        RampVerdict::WithinThreshold { .. }
    ));
}

#[test]
fn the_threshold_boundary_does_not_warn() {
    // Exactly +20% is within; the warning is for exceeding it.
    let week = vec![Some(504)]; // 8.4h == 7h * 1.20
    assert!(
        matches!(
            assess_ramp(&week, Some(&baseline(7.0))),
            RampVerdict::WithinThreshold { .. }
        ),
        "the boundary itself must not fire, or every compliant plan warns"
    );
}

#[test]
fn a_uniformly_hard_bundle_is_caught_by_the_first_week() {
    // Every week identical at 11h on a 6h baseline: no internal ramp at
    // all. This is precisely the plan a week-over-week check passes.
    let flat_week = vec![Some(165), Some(165), Some(165), Some(165)];
    assert!(matches!(
        assess_ramp(&flat_week, Some(&baseline(6.0))),
        RampVerdict::Exceeded { .. }
    ));
}

#[test]
fn an_athlete_with_no_cached_activity_is_reported_not_passed() {
    assert_eq!(
        assess_ramp(&[Some(120)], None),
        RampVerdict::Unmeasurable(RampUnmeasurable::NoBaseline)
    );
}

#[test]
fn a_plan_with_no_durations_is_reported_not_passed() {
    // `duration_min` is optional in the save schema. Summing `None`s to
    // zero would make every unquantified plan look like a rest week and
    // pass silently — the exact way a rail becomes decorative.
    assert_eq!(
        assess_ramp(&[None, None, None], Some(&baseline(7.0))),
        RampVerdict::Unmeasurable(RampUnmeasurable::NoPlannedDurations)
    );
}

#[test]
fn a_partially_quantified_week_is_still_measured() {
    let week = vec![Some(180), None, Some(180)];
    assert!(matches!(
        assess_ramp(&week, Some(&baseline(2.0))),
        RampVerdict::Exceeded { .. }
    ));
}

#[test]
fn a_zero_baseline_cannot_produce_a_ratio() {
    assert_eq!(
        assess_ramp(&[Some(120)], Some(&baseline(0.0))),
        RampVerdict::Unmeasurable(RampUnmeasurable::NoBaseline),
        "dividing by a zero baseline would report an infinite increase"
    );
}

#[test]
fn a_real_rest_week_is_measurable_but_an_unquantified_one_is_not() {
    assert_eq!(planned_week_minutes(&[Some(0), Some(0)]), Some(0));
    assert_eq!(planned_week_minutes(&[None, None]), None);
    assert_eq!(planned_week_minutes(&[]), None);
}

#[test]
fn a_lighter_plan_never_warns() {
    let week = vec![Some(60), Some(60)];
    assert!(matches!(
        assess_ramp(&week, Some(&baseline(7.0))),
        RampVerdict::WithinThreshold { .. }
    ));
}

#[test]
fn unmeasurable_reasons_are_distinguishable_in_the_event() {
    assert_eq!(RampUnmeasurable::NoBaseline.as_str(), "no_baseline");
    assert_eq!(
        RampUnmeasurable::NoPlannedDurations.as_str(),
        "no_planned_durations"
    );
}
