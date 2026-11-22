// ABOUTME: Endurance Phase 1 latest-snapshot export — multi-tenant isolation, shape conformance, and its store read
// ABOUTME: Validates profile scoping, build_latest_snapshot's JSON shape, and that the export counts what a count over its days counts
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use std::sync::Arc;

use dravr_cageux::zones::HrZones;
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::constants::oauth::providers::{GARMIN, STRAVA};
use pierre_core::models::activity::ActivityBuilder;
use pierre_core::models::zones::HrZoneSet;
use pierre_core::models::{
    Activity, AnchorProvenance, ConnectionType, SportType, TenantId, ThresholdProvenance,
    ThresholdStatedOn, UserPhysiologicalProfile,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_database::backends::factory::Database;
use pierre_database::DatabaseProvider;
use pierre_fitness_compute::latest_snapshot::build_latest_snapshot;
use pierre_providers::ai_scope;
use pierre_test_support::db::create_test_db_with_key;
use pierre_tool_runtime::activity_query::{
    query_activities, ActivityPredicate, ActivityQuery, ReadGate, DEFAULT_METRICS,
};
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::json;
use uuid::Uuid;

/// The zones cageux cuts for the profile below (`VO2max` 50, resting 48, max 192).
fn athlete_zones() -> HrZones {
    HrZones::karvonen_tiered(50.0, 48, 192).unwrap()
}

async fn make_test_db() -> Database {
    let encryption_key = b"test_encryption_key_32_bytes_long".to_vec();
    let db = create_test_db_with_key(encryption_key)
        .await
        .expect("create db");
    db.migrate().await.expect("migrate");
    db
}

fn synthetic_run(id: &str, days_ago: i64, np: u32, avg_hr: u32) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        format!("synthetic {id}"),
        SportType::Run,
        Timestamp::now() - Elapsed::days(days_ago),
        3600,
        "synthetic".to_owned(),
    )
    .distance_meters(10_000.0)
    .average_heart_rate(avg_hr)
    .average_power(180)
    .normalized_power(np)
    .build()
}

fn make_profile(
    user_id: Uuid,
    ftp_watts: Option<u32>,
    hr_zones: Option<HrZoneSet>,
) -> UserPhysiologicalProfile {
    UserPhysiologicalProfile {
        user_id,
        vo2_max: Some(50.0),
        resting_hr: Some(48),
        max_hr: Some(192),
        lactate_threshold_percentage: Some(0.85),
        threshold_hr: None,
        age: Some(35),
        weight: Some(70.0),
        fitness_level: FitnessLevel::Intermediate,
        primary_sport: SportType::Run,
        training_experience_years: Some(8),
        ftp_watts,
        threshold_pace_sec_per_km: Some(240.0),
        hr_zones,
        power_zones: None,
        critical_power_watts: None,
        w_prime_joules: None,
        critical_speed_mps: None,
        d_prime_meters: None,
        threshold_provenance: ThresholdProvenance::default(),
        threshold_stated_on: ThresholdStatedOn::default(),
        anchor_provenance: AnchorProvenance::default(),
        transport_policy: TransportPolicy::AnyTransport,
        sex: None,
    }
}

#[tokio::test]
async fn latest_snapshot_returns_endurance_conformant_json() {
    let acts = vec![
        synthetic_run("a1", 1, 200, 150),
        synthetic_run("a2", 2, 210, 155),
    ];
    let zones = athlete_zones();
    let snap = build_latest_snapshot(&acts, 7, Some(250), Ok(&zones));
    let json = serde_json::to_value(&snap).expect("serialize snapshot");

    // Endurance JSON shape — these keys are the contract surface.
    let obj = json.as_object().expect("object root");
    for key in [
        "window_days",
        "window_start",
        "window_end",
        "activity_count",
        "total_distance_meters",
        "total_duration_seconds",
        "avg_intensity_factor",
        "avg_efficiency_factor",
        "avg_variability_index",
        "activities",
    ] {
        assert!(obj.contains_key(key), "missing key: {key}");
    }
    let activities = obj.get("activities").and_then(|v| v.as_array()).unwrap();
    assert_eq!(activities.len(), 2);
    let row = activities[0].as_object().unwrap();
    for key in [
        "activity_id",
        "start_date",
        "sport",
        "duration_seconds",
        "metrics",
    ] {
        assert!(row.contains_key(key), "missing per-activity key: {key}");
    }
}

#[tokio::test]
async fn empty_window_returns_200_with_zero_activities() {
    let zones = athlete_zones();
    let snap = build_latest_snapshot(&[], 7, Some(250), Ok(&zones));
    assert_eq!(snap.activity_count, 0);
    assert!(snap.activities.is_empty());
    assert!(snap.avg_intensity_factor.is_none());
}

#[tokio::test]
async fn user_physiological_profile_is_tenant_scoped() {
    let db = make_test_db().await;
    let user_id = Uuid::new_v4();
    let tenant_a = TenantId::generate();
    let tenant_b = TenantId::generate();

    let profile_a = make_profile(user_id, Some(250), None);
    let profile_b = make_profile(user_id, Some(300), None);

    let repos = db.repositories();
    repos
        .user_physiological_profile
        .upsert_user_physiological_profile(tenant_a, user_id, &profile_a)
        .await
        .expect("upsert tenant A");
    repos
        .user_physiological_profile
        .upsert_user_physiological_profile(tenant_b, user_id, &profile_b)
        .await
        .expect("upsert tenant B");

    let read_a = repos
        .user_physiological_profile
        .get_user_physiological_profile(tenant_a, user_id)
        .await
        .expect("read A")
        .expect("row A present");
    let read_b = repos
        .user_physiological_profile
        .get_user_physiological_profile(tenant_b, user_id)
        .await
        .expect("read B")
        .expect("row B present");
    assert_eq!(read_a.ftp_watts, Some(250));
    assert_eq!(read_b.ftp_watts, Some(300));

    // Cross-tenant lookup with the other tenant's id must miss.
    let other_user = Uuid::new_v4();
    let miss = repos
        .user_physiological_profile
        .get_user_physiological_profile(tenant_a, other_user)
        .await
        .expect("read");
    assert!(miss.is_none(), "lookup for unknown user must be None");
}

#[tokio::test]
async fn upsert_rejects_mismatched_user_id() {
    let db = make_test_db().await;
    let tenant_id = TenantId::generate();
    let row_user = Uuid::new_v4();
    let profile_user = Uuid::new_v4();
    let profile = make_profile(profile_user, Some(250), None);

    let err = db
        .repositories()
        .user_physiological_profile
        .upsert_user_physiological_profile(tenant_id, row_user, &profile)
        .await
        .expect_err("mismatched user_id must fail");
    let msg = format!("{err}");
    assert!(
        msg.contains("does not match"),
        "expected user_id mismatch error, got: {msg}"
    );
}

#[tokio::test]
async fn profile_round_trips_endurance_fields() {
    let db = make_test_db().await;
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4();
    let zones = HrZoneSet::from_zones(&athlete_zones());
    let profile = make_profile(user_id, Some(275), Some(zones));
    let repos = db.repositories();
    repos
        .user_physiological_profile
        .upsert_user_physiological_profile(tenant_id, user_id, &profile)
        .await
        .expect("upsert");
    let read = repos
        .user_physiological_profile
        .get_user_physiological_profile(tenant_id, user_id)
        .await
        .expect("read")
        .expect("row");
    assert_eq!(read.ftp_watts, Some(275));
    assert_eq!(read.threshold_pace_sec_per_km, Some(240.0));
    assert_eq!(read.hr_zones, Some(zones));
}

#[tokio::test]
async fn snapshot_with_zero_activities_does_not_panic_when_zones_supplied() {
    let zones = HrZones::karvonen_tiered(42.0, 55, 190).unwrap();
    let snap = build_latest_snapshot(&[], 7, Some(250), Ok(&zones));
    assert!(snap.aggregated_zone_distribution.is_none());
}

/// One run of `id` recorded by `provider`, started at `start`.
fn stored_run(id: &str, start: Timestamp, provider: &str) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        format!("Run {id}"),
        SportType::Run,
        start,
        3_000,
        provider.to_owned(),
    )
    .distance_meters(8_500.0)
    .build()
}

/// The export reads its days from the store, every source merged, and counts
/// exactly the workouts a count over the same days counts. The athlete's
/// connection holds no credential, so a provider read could only fail: the
/// snapshot answering at all says it read none.
#[tokio::test]
async fn the_snapshot_counts_the_stored_workouts_a_count_over_its_days_counts() {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await.unwrap();
    let email = format!("snapshot-store-{}@example.com", Uuid::new_v4());
    let (user_id, _user, tenant) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .unwrap();
    resources
        .repos()
        .provider_connections
        .register_connection(user_id, tenant, STRAVA, &ConnectionType::OAuth, None)
        .await
        .unwrap();
    // Three runs in the week, each recorded by Strava and by Garmin, and one
    // a month back. The seeded athlete has no timezone, so their calendar is
    // UTC's; noon keeps every run well inside its day.
    let noon = Timestamp::now()
        .utc_date()
        .and_hms(12, 0, 0)
        .unwrap()
        .and_utc()
        .unwrap();
    let cache = &resources.repos().activity_cache;
    for (index, days) in [1, 3, 5, 30].into_iter().enumerate() {
        let start = noon - Elapsed::days(days);
        cache
            .upsert_activities(
                user_id,
                &tenant,
                STRAVA,
                &[stored_run(&format!("s-{index}"), start, STRAVA)],
            )
            .await
            .unwrap();
        cache
            .upsert_activities(
                user_id,
                &tenant,
                GARMIN,
                &[stored_run(&format!("g-{index}"), start, GARMIN)],
            )
            .await
            .unwrap();
    }

    let runtime: Arc<dyn ToolRuntime> = resources;
    // The athlete's own app: Strava's and Garmin's terms keep their data off
    // an external interface, which would withhold every copy from both sides.
    let executor = UniversalToolExecutor::new(Arc::clone(&runtime))
        .with_scopes(OAuthScope::self_grant())
        .with_transport(Transport::WebApp);
    let response = executor
        .execute_tool(UniversalRequest {
            tool_name: "export_latest_snapshot".to_owned(),
            parameters: json!({ "window": 7 }),
            user_id: user_id.to_string(),
            protocol: "test".to_owned(),
            tenant_id: Some(tenant.to_string()),
        })
        .await
        .unwrap_or_else(|e| panic!("bug: the snapshot read a provider, not the store: {e:?}"));
    assert!(
        response.success,
        "bug: the snapshot did not answer from the store: {:?}",
        response.error
    );
    let snapshot = response.result.unwrap();

    let today = Timestamp::now().utc_date();
    let query = ActivityQuery {
        from: Some(today - Elapsed::days(6)),
        to: today,
        filter: ActivityPredicate::default(),
        group_by: Vec::new(),
        metrics: DEFAULT_METRICS.to_vec(),
        provider: None,
    };
    let (answer, _) = ai_scope::serve_over(
        Transport::WebApp,
        ai_scope::ai_read(Box::pin(query_activities(
            &runtime,
            user_id,
            tenant,
            &query,
            ReadGate::Model,
        ))),
    )
    .await;
    let totals = answer.unwrap().totals;

    assert_eq!(
        snapshot["activity_count"], 3,
        "bug: the week's three runs, two copies each, are not three workouts: {snapshot}"
    );
    assert_eq!(
        snapshot["activity_count"].as_u64(),
        totals.count,
        "bug: the snapshot counts other workouts than the count over its days"
    );
    let snapshot_metres = snapshot["total_distance_meters"].as_f64().unwrap();
    let counted_metres = totals.distance_km.unwrap() * 1000.0;
    assert!(
        (snapshot_metres - counted_metres).abs() < 1e-6,
        "bug: the snapshot's distance {snapshot_metres} m is not the count's {counted_metres} m"
    );
    assert_eq!(
        snapshot["total_duration_seconds"].as_u64(),
        totals.elapsed_time_s,
        "bug: the snapshot's duration is not the count's elapsed time"
    );
}
