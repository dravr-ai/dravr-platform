// ABOUTME: Endurance Phase 2 — TrainingHistoryRepository multi-tenant isolation + round-trip + range queries
// ABOUTME: Validates SQLite implementation; PG mirrors the same trait with parametric queries
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{Duration, NaiveDate};
#[cfg(feature = "postgresql")]
use pierre_core::config::database::PostgresPoolConfig;
use pierre_core::models::{DailyTrainingState, TenantId};
use pierre_database::backends::factory::Database;
use pierre_database::DatabaseProvider;
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

fn anchor() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()
}

fn synthetic_state(day_offset: i64, ctl: f64, atl: f64, daily_load: f64) -> DailyTrainingState {
    DailyTrainingState {
        date: anchor() + Duration::days(day_offset),
        ctl,
        atl,
        tsb: ctl - atl,
        acwr: Some(1.0),
        monotony: Some(1.5),
        strain: Some(daily_load * 1.5 * 7.0),
        ramp_rate: Some(0.5),
        daily_load,
    }
}

#[tokio::test]
async fn upsert_and_fetch_single_day_round_trips() {
    let db = make_test_db().await;
    let tenant_id = TenantId::new();
    let user_id = Uuid::new_v4();
    let state = synthetic_state(0, 45.0, 50.0, 80.0);
    db.repositories()
        .training_history
        .upsert_training_history_day(tenant_id, user_id, &state)
        .await
        .expect("upsert");
    let read = db
        .repositories()
        .training_history
        .latest_training_history(tenant_id, user_id)
        .await
        .expect("latest")
        .expect("row present");
    assert_eq!(read.date, state.date);
    assert!((read.ctl - 45.0).abs() < 1e-9);
    assert!((read.atl - 50.0).abs() < 1e-9);
    assert!((read.tsb - (-5.0)).abs() < 1e-9);
    assert_eq!(read.acwr, Some(1.0));
}

#[tokio::test]
async fn upsert_overwrites_existing_day() {
    let db = make_test_db().await;
    let tenant_id = TenantId::new();
    let user_id = Uuid::new_v4();
    let v1 = synthetic_state(0, 40.0, 45.0, 70.0);
    let v2 = synthetic_state(0, 50.0, 55.0, 90.0);
    let repos = db.repositories();
    repos
        .training_history
        .upsert_training_history_day(tenant_id, user_id, &v1)
        .await
        .expect("upsert v1");
    repos
        .training_history
        .upsert_training_history_day(tenant_id, user_id, &v2)
        .await
        .expect("upsert v2");
    let read = repos
        .training_history
        .latest_training_history(tenant_id, user_id)
        .await
        .expect("latest")
        .expect("row");
    assert!((read.ctl - 50.0).abs() < 1e-9);
    assert!((read.daily_load - 90.0).abs() < 1e-9);
}

#[tokio::test]
async fn batch_upsert_and_range_query_return_chronological_order() {
    let db = make_test_db().await;
    let tenant_id = TenantId::new();
    let user_id = Uuid::new_v4();
    let states: Vec<DailyTrainingState> = (0..30)
        .map(|i| {
            synthetic_state(
                i,
                (i as f64).mul_add(0.5, 40.0),
                (i as f64).mul_add(0.4, 45.0),
                80.0,
            )
        })
        .collect();
    let repos = db.repositories();
    repos
        .training_history
        .upsert_training_history_batch(tenant_id, user_id, &states)
        .await
        .expect("batch upsert");
    let from = anchor();
    let to = anchor() + Duration::days(29);
    let rows = repos
        .training_history
        .get_training_history(tenant_id, user_id, from, to)
        .await
        .expect("get history");
    assert_eq!(rows.len(), 30);
    for window in rows.windows(2) {
        assert!(
            window[0].date < window[1].date,
            "rows must be chronological"
        );
    }
}

#[tokio::test]
async fn range_query_excludes_rows_outside_window() {
    let db = make_test_db().await;
    let tenant_id = TenantId::new();
    let user_id = Uuid::new_v4();
    let states: Vec<DailyTrainingState> = (0..20)
        .map(|i| synthetic_state(i, 40.0, 45.0, 80.0))
        .collect();
    db.repositories()
        .training_history
        .upsert_training_history_batch(tenant_id, user_id, &states)
        .await
        .expect("batch upsert");
    let rows = db
        .repositories()
        .training_history
        .get_training_history(
            tenant_id,
            user_id,
            anchor() + Duration::days(5),
            anchor() + Duration::days(10),
        )
        .await
        .expect("range");
    assert_eq!(rows.len(), 6);
    assert_eq!(rows.first().unwrap().date, anchor() + Duration::days(5));
    assert_eq!(rows.last().unwrap().date, anchor() + Duration::days(10));
}

#[tokio::test]
async fn training_history_is_tenant_scoped() {
    let db = make_test_db().await;
    let tenant_a = TenantId::new();
    let tenant_b = TenantId::new();
    let user_id = Uuid::new_v4();
    let state_a = synthetic_state(0, 30.0, 35.0, 60.0);
    let state_b = synthetic_state(0, 70.0, 65.0, 120.0);
    let repos = db.repositories();
    repos
        .training_history
        .upsert_training_history_day(tenant_a, user_id, &state_a)
        .await
        .expect("upsert A");
    repos
        .training_history
        .upsert_training_history_day(tenant_b, user_id, &state_b)
        .await
        .expect("upsert B");
    let read_a = repos
        .training_history
        .latest_training_history(tenant_a, user_id)
        .await
        .expect("read A")
        .expect("row A");
    let read_b = repos
        .training_history
        .latest_training_history(tenant_b, user_id)
        .await
        .expect("read B")
        .expect("row B");
    assert!((read_a.ctl - 30.0).abs() < 1e-9);
    assert!((read_b.ctl - 70.0).abs() < 1e-9);
    // Cross-tenant read with the wrong user must miss.
    let other_user = Uuid::new_v4();
    let miss = repos
        .training_history
        .latest_training_history(tenant_a, other_user)
        .await
        .expect("read")
        .map(|r| r.ctl);
    assert!(miss.is_none());
}

#[tokio::test]
async fn empty_history_returns_none_for_latest() {
    let db = make_test_db().await;
    let latest = db
        .repositories()
        .training_history
        .latest_training_history(TenantId::new(), Uuid::new_v4())
        .await
        .expect("read");
    assert!(latest.is_none());
}
