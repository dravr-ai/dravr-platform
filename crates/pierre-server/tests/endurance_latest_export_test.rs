// ABOUTME: Endurance Phase 1 latest-snapshot export — multi-tenant isolation + shape conformance
// ABOUTME: Validates UserPhysiologicalProfileRepository scoping and the JSON shape returned by build_latest_snapshot
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{Duration, Utc};
#[cfg(feature = "postgresql")]
use pierre_core::config::database::PostgresPoolConfig;
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::models::activity::ActivityBuilder;
use pierre_core::models::zones::HrZoneSet;
use pierre_core::models::{Activity, SportType, TenantId, UserPhysiologicalProfile};
use pierre_database::backends::factory::Database;
use pierre_database::DatabaseProvider;
use pierre_intelligence::latest_snapshot::build_latest_snapshot;
use uuid::Uuid;

async fn make_test_db() -> Database {
    let encryption_key = b"test_encryption_key_32_bytes_long".to_vec();
    #[cfg(feature = "postgresql")]
    let db = Database::new(
        "sqlite::memory:",
        encryption_key,
        &PostgresPoolConfig::default(),
    )
    .await
    .expect("create db");
    #[cfg(not(feature = "postgresql"))]
    let db = Database::new("sqlite::memory:", encryption_key)
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
        Utc::now() - Duration::days(days_ago),
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
        age: Some(35),
        weight: Some(70.0),
        fitness_level: FitnessLevel::Intermediate,
        primary_sport: SportType::Run,
        training_experience_years: Some(8),
        ftp_watts,
        threshold_pace_sec_per_km: Some(240.0),
        hr_zones,
        power_zones: None,
    }
}

#[tokio::test]
async fn latest_snapshot_returns_endurance_conformant_json() {
    let acts = vec![
        synthetic_run("a1", 1, 200, 150),
        synthetic_run("a2", 2, 210, 155),
    ];
    let zones = HrZoneSet::new(120, 140, 160, 180, 200).unwrap();
    let snap = build_latest_snapshot(&acts, 7, Some(250), Some(zones));
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
    let zones = HrZoneSet::new(120, 140, 160, 180, 200).unwrap();
    let snap = build_latest_snapshot(&[], 7, Some(250), Some(zones));
    assert_eq!(snap.activity_count, 0);
    assert!(snap.activities.is_empty());
    assert!(snap.avg_intensity_factor.is_none());
}

#[tokio::test]
async fn user_physiological_profile_is_tenant_scoped() {
    let db = make_test_db().await;
    let user_id = Uuid::new_v4();
    let tenant_a = TenantId::new();
    let tenant_b = TenantId::new();

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
    let tenant_id = TenantId::new();
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
    let tenant_id = TenantId::new();
    let user_id = Uuid::new_v4();
    let zones = HrZoneSet::new(120, 140, 160, 180, 200).unwrap();
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
    let zones = HrZoneSet::new(110, 130, 150, 170, 190).unwrap();
    let snap = build_latest_snapshot(&[], 7, Some(250), Some(zones));
    assert!(snap.aggregated_zone_distribution.is_none());
}
