// ABOUTME: Unit tests for the commitment sweep's pure decision functions
// ABOUTME: Verdict thresholds, duplicate-session collapsing, and the data-freshness guard
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{Duration, TimeZone, Utc};
use pierre_memory::commitments::CommitmentOutcome;
use pierre_services::commitment_sweep::{
    commitment_outcome, count_sessions, data_covers_window, DUPLICATE_SESSION_WINDOW_SECS,
};

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
fn sessions_are_counted_not_summed_from_rows() {
    let base = Utc.with_ymd_and_hms(2026, 8, 10, 7, 0, 0).unwrap();
    let starts = vec![base, base + Duration::days(2), base + Duration::days(4)];
    assert_eq!(count_sessions(&starts), 3);
    assert_eq!(count_sessions(&[]), 0, "an empty window counts nothing");
}

#[test]
fn two_providers_recording_one_run_count_once() {
    let base = Utc.with_ymd_and_hms(2026, 8, 10, 7, 0, 0).unwrap();
    // Strava and Garmin both cached the same morning run, seconds apart.
    let starts = vec![base, base + Duration::seconds(45)];
    assert_eq!(
        count_sessions(&starts),
        1,
        "the same run from two providers is one session, not two"
    );
}

#[test]
fn a_double_day_still_counts_twice() {
    let base = Utc.with_ymd_and_hms(2026, 8, 10, 7, 0, 0).unwrap();
    let starts = vec![base, base + Duration::hours(11)];
    assert_eq!(
        count_sessions(&starts),
        2,
        "a genuine morning and evening session are two"
    );
}

#[test]
fn dedup_boundary_is_exclusive() {
    let base = Utc.with_ymd_and_hms(2026, 8, 10, 7, 0, 0).unwrap();
    let inside = vec![
        base,
        base + Duration::seconds(DUPLICATE_SESSION_WINDOW_SECS - 1),
    ];
    let outside = vec![
        base,
        base + Duration::seconds(DUPLICATE_SESSION_WINDOW_SECS),
    ];
    assert_eq!(count_sessions(&inside), 1);
    assert_eq!(count_sessions(&outside), 2);
}

#[test]
fn unsorted_starts_still_collapse() {
    let base = Utc.with_ymd_and_hms(2026, 8, 10, 7, 0, 0).unwrap();
    // Cached activities come back newest-first, so the input is not ascending.
    let starts = vec![base + Duration::days(2), base + Duration::seconds(30), base];
    assert_eq!(count_sessions(&starts), 2);
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
