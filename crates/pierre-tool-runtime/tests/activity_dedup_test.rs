// ABOUTME: Regression tests for cross-provider activity deduplication
// ABOUTME: Pins that the same workout synced from two providers counts once
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Tests for [`pierre_tool_runtime::group_fitness::TimeWindowDeduplicator`].
//!
//! A workout recorded on one device is frequently synced from two providers
//! (e.g. a ride on Garmin that is also pushed to Strava). Both the live-fetch
//! path (`AllProvidersMerge::fetch_and_merge`) and the warm-cache snapshot path
//! (`fetch_member_activities`) run the merged list through this deduplicator so
//! the coach sees one activity, not two. These tests pin that collapse.

use chrono::{Duration, TimeZone, Utc};
use pierre_core::models::{ActivityBuilder, SportType};
use pierre_tool_runtime::group_fitness::{ActivityDeduplicator, TimeWindowDeduplicator};

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 6, 6, 12, 0, 0).unwrap()
}

#[test]
fn same_ride_from_two_providers_counts_once() {
    let start = now();
    // The "Phil ride" scenario: 3.2h / 84.8 km cycling synced from both Strava
    // and a Garmin mirror. Same sport, same start, distance within tolerance.
    let activities = vec![
        ActivityBuilder::new(
            "strava-1",
            "Saturday ride",
            SportType::Ride,
            start,
            11_520,
            "strava",
        )
        .distance_meters(84_800.0)
        .build(),
        ActivityBuilder::new(
            "garmin-1",
            "Saturday ride",
            SportType::Ride,
            start + Duration::minutes(2),
            11_520,
            "garmin",
        )
        .distance_meters(84_750.0)
        .build(),
    ];

    let merged = TimeWindowDeduplicator::from_env().deduplicate(activities);

    assert_eq!(
        merged.len(),
        1,
        "the same ride from two providers must collapse to one"
    );
}

#[test]
fn distinct_rides_same_day_are_both_kept() {
    let start = now();
    // Two genuinely different rides on the same day (morning + evening) must not
    // be collapsed — different distance, hours apart.
    let activities = vec![
        ActivityBuilder::new(
            "strava-morning",
            "Morning ride",
            SportType::Ride,
            start,
            3600,
            "strava",
        )
        .distance_meters(30_000.0)
        .build(),
        ActivityBuilder::new(
            "garmin-evening",
            "Evening ride",
            SportType::Ride,
            start + Duration::hours(8),
            5400,
            "garmin",
        )
        .distance_meters(60_000.0)
        .build(),
    ];

    let merged = TimeWindowDeduplicator::from_env().deduplicate(activities);

    assert_eq!(
        merged.len(),
        2,
        "two distinct rides on the same day are not duplicates"
    );
}

#[test]
fn same_provider_near_identical_activities_are_not_deduped() {
    let start = now();
    // Dedup is cross-provider only: a single provider's own rows are trusted as
    // distinct (the cache upsert keys on activity id, so true intra-provider
    // duplicates cannot occur).
    let activities = vec![
        ActivityBuilder::new("strava-a", "Lap 1", SportType::Ride, start, 3600, "strava")
            .distance_meters(20_000.0)
            .build(),
        ActivityBuilder::new(
            "strava-b",
            "Lap 2",
            SportType::Ride,
            start + Duration::minutes(1),
            3600,
            "strava",
        )
        .distance_meters(20_000.0)
        .build(),
    ];

    let merged = TimeWindowDeduplicator::from_env().deduplicate(activities);

    assert_eq!(
        merged.len(),
        2,
        "same-provider activities are never treated as cross-provider duplicates"
    );
}
