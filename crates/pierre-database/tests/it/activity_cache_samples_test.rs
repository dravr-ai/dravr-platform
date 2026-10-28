// ABOUTME: Covers that a cached_activities row never stores an activity's per-sample streams
// ABOUTME: A streams-bearing Activity round-trips through the upsert with its summary and without its samples

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Samples belong to the one per-activity sample store; a provider that
//! attaches them on its list path (sciotte with
//! `PIERRE_SCIOTTE_ENRICH_DETAILS=true`) must not persist them into the
//! summary row's `data_json` (carnet#1220). `create_test_db` opens whichever
//! `DATABASE_URL` names, so the same assertions cover `SQLite` and
//! `PostgreSQL`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{Duration, TimeZone, Utc};
use pierre_core::models::{ActivityBuilder, SportType, TenantId, TimeSeriesData, User};
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

#[tokio::test]
async fn a_streams_bearing_activity_is_cached_without_its_samples() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user = User::new(
        format!("samples-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Sample Store Tester".to_owned()),
    );
    let user_id = repos.users.create(&user).await.unwrap();
    let tenant_id = TenantId::generate();
    let start = Utc.with_ymd_and_hms(2026, 10, 27, 7, 0, 0).unwrap();
    let activity = ActivityBuilder::new(
        "route-1",
        "Lunch run",
        SportType::Run,
        start,
        1_800,
        "sciotte_strava",
    )
    .distance_meters(5_000.0)
    .time_series_data(TimeSeriesData {
        timestamps: vec![0, 1, 2],
        heart_rate: Some(vec![Some(140), Some(141), Some(142)]),
        power: None,
        cadence: None,
        speed: None,
        altitude: Some(vec![Some(50.0), Some(51.0), Some(52.0)]),
        temperature: None,
        gps_coordinates: Some(vec![(45.5, -73.5), (45.6, -73.6), (45.7, -73.7)]),
        distance: None,
    })
    .build();

    repos
        .activity_cache
        .upsert_activities(user_id, &tenant_id, "strava", &[activity])
        .await
        .unwrap();

    let cached = repos
        .activity_cache
        .get_cached_activities(
            user_id,
            &tenant_id,
            Some("strava"),
            start - Duration::days(1),
            start + Duration::days(1),
            10,
        )
        .await
        .unwrap();
    assert_eq!(cached.len(), 1, "the summary row is stored");
    let stored = &cached[0];
    assert_eq!(stored.id(), "route-1", "the summary keeps its id");
    assert_eq!(
        stored.distance_meters(),
        Some(5_000.0),
        "the summary keeps its distance"
    );
    assert!(
        stored.time_series_data().is_none(),
        "the samples stayed out of data_json"
    );
}
