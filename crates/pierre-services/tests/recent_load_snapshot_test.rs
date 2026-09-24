// ABOUTME: Unit tests for the recent-load snapshot built from activity durations
// ABOUTME: Pins averaging over the whole window and the empty and zero-week refusals

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used)]

use pierre_services::recent_load::{snapshot_from_durations, SNAPSHOT_WEEKS};

#[test]
fn an_empty_window_yields_no_baseline_rather_than_zeroes() {
    // A zeroed snapshot would have the agent tell an athlete with no
    // connected provider that they train zero hours a week.
    assert!(snapshot_from_durations(&[], SNAPSHOT_WEEKS).is_none());
}

#[test]
fn averages_are_taken_over_the_window_not_the_active_weeks() {
    // Twelve one-hour sessions over six weeks is two hours a week, even
    // though they all happened in one week. Dividing by "weeks that had a
    // session" would report twelve.
    let snapshot = snapshot_from_durations(&[3600; 12], 6);
    assert_eq!(
        snapshot.as_ref().map(|s| s.weeks),
        Some(6),
        "twelve sessions is a window"
    );
    let within = snapshot.is_some_and(|s| {
        (s.weekly_hours - 2.0).abs() < f64::EPSILON
            && (s.sessions_per_week - 2.0).abs() < f64::EPSILON
    });
    assert!(
        within,
        "twelve hours over six weeks is 2 h/wk across 2 sessions"
    );
}

#[test]
fn the_longest_session_is_the_real_maximum_in_minutes() {
    let snapshot = snapshot_from_durations(&[1800, 11_700, 3600], 6);
    assert_eq!(
        snapshot.map(|s| s.longest_session_min),
        Some(195),
        "3h15 is the longest of the three"
    );
}

#[test]
fn a_single_short_session_still_produces_an_honest_snapshot() {
    let snapshot = snapshot_from_durations(&[1800], 6);
    assert_eq!(snapshot.as_ref().map(|s| s.longest_session_min), Some(30));
    assert!(
        snapshot.is_some_and(|s| s.weekly_hours > 0.0 && s.sessions_per_week < 1.0),
        "one session across six weeks is positive but well under one per week"
    );
}

#[test]
fn a_zero_week_window_is_rejected_rather_than_dividing_by_zero() {
    assert!(snapshot_from_durations(&[3600], 0).is_none());
}
