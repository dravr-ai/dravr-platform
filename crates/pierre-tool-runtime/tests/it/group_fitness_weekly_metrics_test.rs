// ABOUTME: Unit tests for the canonical group-member weekly metrics computation
// ABOUTME: Covers the previous-week volume window that feeds the group weekly trend
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::cast_precision_loss)]
#![allow(missing_docs)]

//! Tests for [`pierre_tool_runtime::group_fitness::compute_weekly_metrics`].
//!
//! The REST group analytics path and the chat agent path both build member
//! fitness snapshots from this single function, so its `previous_week_volume_km`
//! window (7-to-14 days back) must be populated for the group weekly trend to
//! work. Before consolidation only the (now-deleted) REST builder computed it;
//! these tests pin the canonical builder's behavior.

use dravr_tronc::dates::{Elapsed, Timestamp, Zone};
use pierre_core::models::{ActivityBuilder, SportType};
use pierre_tool_runtime::group_fitness::compute_weekly_metrics;

fn now() -> Timestamp {
    Timestamp::parse_from_rfc3339("2026-05-22T12:00:00Z").unwrap()
}

#[test]
fn previous_week_volume_is_some_when_prior_week_has_activity() {
    let now = now();
    let activities = vec![
        // Current week: 3 days ago, 10 km
        ActivityBuilder::new(
            "cur",
            "This week ride",
            SportType::Ride,
            now - Elapsed::days(3),
            3600,
            "strava",
        )
        .distance_meters(10_000.0)
        .build(),
        // Previous week: 10 days ago, 25 km
        ActivityBuilder::new(
            "prev",
            "Last week ride",
            SportType::Ride,
            now - Elapsed::days(10),
            5400,
            "strava",
        )
        .distance_meters(25_000.0)
        .build(),
    ];

    let metrics = compute_weekly_metrics(&activities, now, &Zone::utc());

    assert!((metrics.volume_km - 10.0).abs() < 1e-6, "current week km");
    assert_eq!(
        metrics.activity_count, 1,
        "only current-week activity counted"
    );
    assert_eq!(
        metrics.previous_week_volume_km,
        Some(25.0),
        "prior-week volume must be populated for trend detection"
    );
}

#[test]
fn previous_week_volume_is_none_when_no_prior_week_activity() {
    let now = now();
    let activities = vec![ActivityBuilder::new(
        "cur",
        "This week run",
        SportType::Run,
        now - Elapsed::days(2),
        1800,
        "strava",
    )
    .distance_meters(5000.0)
    .build()];

    let metrics = compute_weekly_metrics(&activities, now, &Zone::utc());

    assert_eq!(
        metrics.previous_week_volume_km, None,
        "absent prior week is a missing trend signal, not a zero week"
    );
}

#[test]
fn activity_older_than_two_weeks_is_excluded_from_both_windows() {
    let now = now();
    let activities = vec![ActivityBuilder::new(
        "old",
        "Three weeks ago",
        SportType::Run,
        now - Elapsed::days(21),
        1800,
        "strava",
    )
    .distance_meters(8000.0)
    .build()];

    let metrics = compute_weekly_metrics(&activities, now, &Zone::utc());

    assert!(
        (metrics.volume_km - 0.0).abs() < 1e-6,
        "no current-week volume"
    );
    assert_eq!(metrics.activity_count, 0);
    assert_eq!(
        metrics.previous_week_volume_km, None,
        "activity outside the 14-day window contributes to neither window"
    );
}
