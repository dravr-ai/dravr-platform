// ABOUTME: Unit tests for the outcome evaluator's pure labeling helpers (no DB / LLM needed)
// ABOUTME: Proves the hybrid labeler's decision logic — adherence, deltas, ramp ceiling, consistency, dead-bands
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Unit tests for the outcome evaluator's pure labeling helpers.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_memory::playbooks::OutcomeLabel;
use pierre_services::outcome_evaluator::{
    activity_completed_label, consistency_label, delta_label, ramp_within_label,
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
