// ABOUTME: Endurance Phase 5 — PrescribedWorkoutRepository round trip + multi-tenant isolation + audit trail
// ABOUTME: Validates upsert + list_recent semantics on the SQLite tier; PG mirrors via the same trait
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::time::Duration;

use chrono::{NaiveDate, Utc};
#[cfg(feature = "postgresql")]
use pierre_core::config::database::PostgresPoolConfig;
use pierre_core::models::{PrescribedWorkout, SportType, TenantId};
use pierre_database::backends::factory::Database;
use pierre_database::DatabaseProvider;
use tokio::time::sleep;
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

fn anchor_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 5, 1).unwrap()
}

fn make_prescribed(
    tenant_id: TenantId,
    user_id: Uuid,
    template_slug: &str,
    date: NaiveDate,
) -> PrescribedWorkout {
    PrescribedWorkout {
        id: Uuid::new_v4(),
        tenant_id: tenant_id.as_uuid(),
        user_id,
        coach_id: Some("endurance-coach".to_owned()),
        template_slug: template_slug.to_owned(),
        sport: SportType::Run,
        prescribed_for_date: date,
        provider: "intervals_icu".to_owned(),
        provider_event_id: None,
        payload_json: r#"{"slug":"long_run_z2"}"#.to_owned(),
        status: "queued".to_owned(),
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn upsert_and_list_round_trips() {
    let db = make_test_db().await;
    let tenant_id = TenantId::new();
    let user_id = Uuid::new_v4();
    let prescribed = make_prescribed(tenant_id, user_id, "long_run_z2", anchor_date());
    db.repositories()
        .prescribed_workouts
        .upsert_prescribed_workout(&prescribed)
        .await
        .expect("upsert");
    let rows = db
        .repositories()
        .prescribed_workouts
        .list_prescribed_workouts(tenant_id, user_id, 10)
        .await
        .expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].template_slug, "long_run_z2");
    assert_eq!(rows[0].status, "queued");
    assert_eq!(rows[0].coach_id.as_deref(), Some("endurance-coach"));
}

#[tokio::test]
async fn upsert_replays_with_same_id_to_update_provider_event_id() {
    let db = make_test_db().await;
    let tenant_id = TenantId::new();
    let user_id = Uuid::new_v4();
    let mut prescribed = make_prescribed(tenant_id, user_id, "threshold_4x8", anchor_date());
    db.repositories()
        .prescribed_workouts
        .upsert_prescribed_workout(&prescribed)
        .await
        .expect("upsert v1");
    prescribed.provider_event_id = Some("intervals-evt-99".to_owned());
    prescribed.status = "pushed".to_owned();
    db.repositories()
        .prescribed_workouts
        .upsert_prescribed_workout(&prescribed)
        .await
        .expect("upsert v2");
    let rows = db
        .repositories()
        .prescribed_workouts
        .list_prescribed_workouts(tenant_id, user_id, 10)
        .await
        .expect("list");
    assert_eq!(
        rows.len(),
        1,
        "same-id upsert must not create a duplicate row"
    );
    assert_eq!(
        rows[0].provider_event_id.as_deref(),
        Some("intervals-evt-99")
    );
    assert_eq!(rows[0].status, "pushed");
}

#[tokio::test]
async fn prescriptions_are_tenant_scoped() {
    let db = make_test_db().await;
    let tenant_a = TenantId::new();
    let tenant_b = TenantId::new();
    let user_id = Uuid::new_v4();
    let p_a = make_prescribed(tenant_a, user_id, "long_run_z2", anchor_date());
    let p_b = make_prescribed(tenant_b, user_id, "vo2_5x3", anchor_date());
    let repos = db.repositories();
    repos
        .prescribed_workouts
        .upsert_prescribed_workout(&p_a)
        .await
        .expect("upsert A");
    repos
        .prescribed_workouts
        .upsert_prescribed_workout(&p_b)
        .await
        .expect("upsert B");

    let rows_a = repos
        .prescribed_workouts
        .list_prescribed_workouts(tenant_a, user_id, 10)
        .await
        .expect("list A");
    let rows_b = repos
        .prescribed_workouts
        .list_prescribed_workouts(tenant_b, user_id, 10)
        .await
        .expect("list B");
    assert_eq!(rows_a.len(), 1);
    assert_eq!(rows_b.len(), 1);
    assert_eq!(rows_a[0].template_slug, "long_run_z2");
    assert_eq!(rows_b[0].template_slug, "vo2_5x3");

    let other_user = Uuid::new_v4();
    let rows_c = repos
        .prescribed_workouts
        .list_prescribed_workouts(tenant_a, other_user, 10)
        .await
        .expect("list C");
    assert!(rows_c.is_empty(), "other-user list must be empty");
}

#[tokio::test]
async fn list_orders_newest_first_and_respects_limit() {
    let db = make_test_db().await;
    let tenant_id = TenantId::new();
    let user_id = Uuid::new_v4();
    let repos = db.repositories();
    for i in 0..5 {
        let prescribed = make_prescribed(
            tenant_id,
            user_id,
            "recovery_30min",
            anchor_date() + chrono::Duration::days(i),
        );
        repos
            .prescribed_workouts
            .upsert_prescribed_workout(&prescribed)
            .await
            .expect("upsert");
        // Tiny delay to ensure distinct created_at values.
        sleep(Duration::from_millis(5)).await;
    }
    let rows = repos
        .prescribed_workouts
        .list_prescribed_workouts(tenant_id, user_id, 3)
        .await
        .expect("list");
    assert_eq!(rows.len(), 3, "limit must cap the returned rows");
    assert!(
        rows.windows(2).all(|w| w[0].created_at >= w[1].created_at),
        "rows must be newest-first"
    );
}
