// ABOUTME: Unit tests for the outcome evaluator's pure labeling helpers (no DB / LLM needed)
// ABOUTME: Proves the hybrid labeler's decision logic — adherence, deltas, ramp ceiling, consistency, dead-bands
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Unit tests for the outcome evaluator's pure labeling helpers.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::iter;

use pierre_memory::playbooks::OutcomeLabel;
use pierre_services::outcome_evaluator::{
    activity_completed_label, consistency_label, delta_label, peak_atl_ctl_ratio, ramp_within_label,
};

#[test]
fn activity_adherence_labels() {
    assert_eq!(
        activity_completed_label(true, true),
        Some(OutcomeLabel::Success)
    );
    // Did other activities but not the suggested one -> failure (adherence miss).
    assert_eq!(
        activity_completed_label(false, true),
        Some(OutcomeLabel::Failure)
    );
    // No activity data at all -> cannot tell -> expire.
    assert_eq!(activity_completed_label(false, false), None);
}

#[test]
fn delta_orientation_and_dead_band() {
    // Higher-is-better, clear improvement -> Success, not ambiguous.
    let up = delta_label(Some(50.0), Some(60.0), true, 3.0);
    assert_eq!(up.label, Some(OutcomeLabel::Success));
    assert!(!up.ambiguous);

    // Higher-is-better, clear decline -> Failure.
    let down = delta_label(Some(60.0), Some(50.0), true, 3.0);
    assert_eq!(down.label, Some(OutcomeLabel::Failure));

    // Within the dead-band -> Neutral AND ambiguous (escalate to the judge).
    let flat = delta_label(Some(50.0), Some(51.0), true, 3.0);
    assert_eq!(flat.label, Some(OutcomeLabel::Neutral));
    assert!(flat.ambiguous);

    // Lower-is-better flips orientation: a drop is a Success.
    let lower_better = delta_label(Some(60.0), Some(50.0), false, 3.0);
    assert_eq!(lower_better.label, Some(OutcomeLabel::Success));

    // Missing an endpoint -> no delta -> expire.
    assert_eq!(delta_label(None, Some(50.0), true, 3.0).label, None);
}

#[test]
fn a_building_week_is_judged_on_its_ratio_not_its_ctl_gain() {
    // CTL climbs 40 -> 48 in a week (a ramp of +8 points) while ATL stays
    // within 1.3 of it: the window is within the ceiling.
    let days = [
        (44.0, 40.0),
        (48.0, 42.0),
        (52.0, 44.0),
        (57.0, 46.0),
        (60.0, 48.0),
    ];
    let peak = peak_atl_ctl_ratio(days).expect("bug: a window with a chronic base has a peak");
    assert!(
        (peak - 1.25).abs() < 1e-12,
        "bug: the peak is not the highest daily ATL/CTL, got {peak}"
    );
    assert_eq!(
        ramp_within_label(peak, 1.3),
        OutcomeLabel::Success,
        "bug: a CTL gain under the ratio ceiling is judged as past it"
    );
}

#[test]
fn one_day_past_the_ceiling_fails_the_window() {
    // CTL barely moves, but one day's ATL reaches 1.4 x CTL.
    let days = [(40.0, 40.0), (56.0, 40.0), (44.0, 40.5)];
    let peak = peak_atl_ctl_ratio(days).expect("bug: a window with a chronic base has a peak");
    assert!(
        (peak - 1.4).abs() < 1e-12,
        "bug: the peak is not the highest daily ATL/CTL, got {peak}"
    );
    assert_eq!(
        ramp_within_label(peak, 1.3),
        OutcomeLabel::Failure,
        "bug: one day past the ratio ceiling does not fail the window"
    );
}

#[test]
fn a_window_with_no_chronic_base_has_no_peak() {
    assert_eq!(
        peak_atl_ctl_ratio([(5.0, 0.0), (0.0, 0.0)]),
        None,
        "bug: a window with no chronic base reports a peak"
    );
    assert_eq!(
        peak_atl_ctl_ratio(iter::empty()),
        None,
        "bug: an empty window reports a peak"
    );
    // A day with no base is skipped, not read as an infinite ratio.
    assert_eq!(
        peak_atl_ctl_ratio([(5.0, 0.0), (10.0, 10.0)]),
        Some(1.0),
        "bug: a day with no chronic base is not skipped"
    );
}

#[test]
fn ramp_ceiling_is_inclusive() {
    assert_eq!(ramp_within_label(1.1, 1.3), OutcomeLabel::Success);
    assert_eq!(ramp_within_label(1.5, 1.3), OutcomeLabel::Failure);
    // At the ceiling counts as within.
    assert_eq!(ramp_within_label(1.3, 1.3), OutcomeLabel::Success);
}

#[test]
fn consistency_count_thresholds() {
    assert_eq!(consistency_label(4, 2).label, Some(OutcomeLabel::Success));
    assert_eq!(consistency_label(0, 2).label, Some(OutcomeLabel::Failure));
    // Between zero and target -> ambiguous Neutral.
    let mid = consistency_label(1, 3);
    assert_eq!(mid.label, Some(OutcomeLabel::Neutral));
    assert!(mid.ambiguous);
    // Expected target floors at 1.
    assert_eq!(consistency_label(1, 0).label, Some(OutcomeLabel::Success));
}
