// ABOUTME: Unit tests for the commitment sweep's pure decision functions
// ABOUTME: Verdict thresholds and the data-freshness guard
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{Duration, TimeZone, Utc};
use pierre_memory::commitments::CommitmentOutcome;
use pierre_services::commitment_sweep::{commitment_outcome, data_covers_window};

#[test]
fn verdict_thresholds() {
    assert_eq!(commitment_outcome(3, 3), CommitmentOutcome::Met);
    assert_eq!(
        commitment_outcome(4, 3),
        CommitmentOutcome::Met,
        "over-delivering is still met, never a separate failure"
    );
    assert_eq!(
        commitment_outcome(2, 3),
        CommitmentOutcome::Partial,
        "two of three is the case the whole feature exists for"
    );
    assert_eq!(commitment_outcome(1, 3), CommitmentOutcome::Partial);
    assert_eq!(commitment_outcome(0, 3), CommitmentOutcome::Missed);
}

#[test]
fn zero_target_cannot_accuse_the_athlete() {
    // A corrupt or unvalidated row must not read as "missed" — that would tell
    // someone they failed a promise they never made.
    assert_eq!(commitment_outcome(0, 0), CommitmentOutcome::Met);
}

#[test]
fn freshness_guard_needs_a_sync_past_the_window() {
    let window_end = Utc.with_ymd_and_hms(2026, 8, 16, 23, 59, 59).unwrap();

    assert!(
        !data_covers_window(None, window_end),
        "an athlete whose cache was never warmed has not been proven to miss anything"
    );
    assert!(
        !data_covers_window(Some(window_end - Duration::hours(6)), window_end),
        "a sync that predates the window close cannot rule out a late session"
    );
    assert!(data_covers_window(Some(window_end), window_end));
    assert!(data_covers_window(
        Some(window_end + Duration::hours(2)),
        window_end
    ));
}
