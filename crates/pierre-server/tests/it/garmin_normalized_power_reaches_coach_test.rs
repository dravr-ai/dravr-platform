// ABOUTME: A Garmin activity's normalized power survives the provider read into the coach-facing summary
// ABOUTME: And with it the intensity, efficiency and variability figures the latest snapshot derives from NP
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin NP, provider to coach (carnet#105).
//!
//! A Garmin ride once reached the agent with no NP and `None` for IF / EF /
//! VI. The Garmin provider now serves what Garmin pushed to Dravr, so this
//! stores a pushed ride carrying its NP in the activity cache, reads it back
//! through the real `GarminProvider` bound to the athlete's stores, and
//! follows the resulting `Activity` into the two places the coach reads it:
//! the `ActivitySummary` the activity tools serialize, and the endurance
//! snapshot's derived factors.
#![cfg(feature = "provider-garmin")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use chrono::{Duration, Utc};
use pierre_core::models::zones::HrZonesUnavailable;
use pierre_core::models::{Activity, ActivityBuilder, SportType, TenantId};
use pierre_fitness_compute::latest_snapshot::build_latest_snapshot;
use pierre_mcp_server::constants::oauth_providers;
use pierre_providers::core::{FitnessProvider, OwnerStores};
use pierre_providers::garmin_provider::GarminProvider;
use pierre_tool_runtime::implementations::activity_summary::ActivitySummary;
use std::sync::Arc;

const RIDE_ID: &str = "20260427001";

/// A pushed cycling activity with every power figure Garmin reports.
fn pushed_ride() -> Activity {
    ActivityBuilder::new(
        RIDE_ID,
        "Tempo ride",
        SportType::Ride,
        Utc::now() - Duration::days(1),
        5_400,
        oauth_providers::GARMIN,
    )
    .source(oauth_providers::GARMIN)
    .distance_meters(42_000.0)
    .average_heart_rate(148)
    .average_power(214)
    .max_power(640)
    .normalized_power(231)
    .build()
}

/// The real Garmin provider, bound to an athlete's stores holding the
/// pushed ride.
async fn provider_serving_the_pushed_ride() -> GarminProvider {
    common::init_server_config();
    common::init_test_http_clients();
    let database = common::create_test_database().await.unwrap();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let tenant = TenantId::generate();
    let repos = database.repositories();
    repos
        .activity_cache
        .upsert_activities(user_id, &tenant, oauth_providers::GARMIN, &[pushed_ride()])
        .await
        .unwrap();

    let provider = GarminProvider::new();
    provider.bind_owner_stores(OwnerStores {
        activity_cache: Arc::clone(&repos.activity_cache),
        activity_streams: Arc::clone(&repos.activity_streams),
        user_id,
        tenant_id: tenant,
    });
    provider
}

#[tokio::test]
async fn a_garmin_rides_np_reaches_the_coach_summary() {
    let provider = provider_serving_the_pushed_ride().await;

    let activity = provider
        .get_activity(RIDE_ID)
        .await
        .expect("the pushed ride is read");

    let summary = ActivitySummary::from(&activity);
    assert_eq!(summary.average_power, Some(214));
    assert_eq!(
        summary.normalized_power,
        Some(231),
        "the NP Garmin reported is what the agent reads, not the average and not nothing"
    );
}

#[tokio::test]
async fn a_garmin_rides_np_populates_if_ef_and_vi() {
    let provider = provider_serving_the_pushed_ride().await;
    let activity = provider
        .get_activity(RIDE_ID)
        .await
        .expect("the pushed ride is read");

    let snapshot = build_latest_snapshot(
        &[activity],
        7,
        Some(250),
        Err(&HrZonesUnavailable::NoProfile),
    );

    let metrics = snapshot.activities.first().expect("one row").metrics;
    let close = |actual: Option<f64>, expected: f64| {
        let actual = actual.expect("the factor is derived from the NP");
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    };
    close(metrics.intensity_factor, 231.0 / 250.0);
    close(metrics.efficiency_factor, 231.0 / 148.0);
    close(metrics.variability_index, 231.0 / 214.0);
}
