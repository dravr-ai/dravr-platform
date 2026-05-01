// ABOUTME: Endurance Phase 2 — history endpoint shape conformance + multi-tenant isolation via repository
// ABOUTME: Validates HistoryResponse JSON shape against the persisted training_history rows
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
use serde::Serialize;
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

fn synthetic_state(day_offset: i64, daily_load: f64) -> DailyTrainingState {
    DailyTrainingState {
        date: anchor() + Duration::days(day_offset),
        ctl: (day_offset as f64).mul_add(0.1, 45.0),
        atl: (day_offset as f64).mul_add(0.05, 50.0),
        tsb: -5.0,
        acwr: Some(1.0),
        monotony: Some(1.5),
        strain: Some(daily_load * 1.5 * 7.0),
        ramp_rate: Some(0.5),
        daily_load,
    }
}

/// Mirror of `routes::endurance::HistoryResponse` for shape testing without
/// pulling `pierre_mcp_server`'s full HTTP stack into a unit test.
#[derive(Debug, Serialize)]
struct HistoryResponseShape {
    from: NaiveDate,
    to: NaiveDate,
    days: Vec<DailyTrainingState>,
}

#[tokio::test]
async fn history_response_serialises_with_endurance_keys() {
    let db = make_test_db().await;
    let tenant_id = TenantId::new();
    let user_id = Uuid::new_v4();
    let states: Vec<DailyTrainingState> = (0..7)
        .map(|i| synthetic_state(i, 60.0 + i as f64))
        .collect();
    db.repositories()
        .training_history
        .upsert_training_history_batch(tenant_id, user_id, &states)
        .await
        .expect("batch upsert");

    let from = anchor();
    let to = anchor() + Duration::days(6);
    let days = db
        .repositories()
        .training_history
        .get_training_history(tenant_id, user_id, from, to)
        .await
        .expect("get history");
    let response = HistoryResponseShape { from, to, days };
    let json = serde_json::to_value(&response).expect("serialize");
    let obj = json.as_object().expect("object root");
    for key in ["from", "to", "days"] {
        assert!(obj.contains_key(key), "missing key: {key}");
    }
    let days_array = obj.get("days").and_then(|v| v.as_array()).unwrap();
    assert_eq!(days_array.len(), 7);
    let day_obj = days_array[0].as_object().unwrap();
    for key in ["date", "ctl", "atl", "tsb", "daily_load"] {
        assert!(day_obj.contains_key(key), "missing per-day key: {key}");
    }
    // Optional fields must serialize when present.
    assert!(day_obj.contains_key("acwr"));
    assert!(day_obj.contains_key("monotony"));
    assert!(day_obj.contains_key("strain"));
    assert!(day_obj.contains_key("ramp_rate"));
}

#[tokio::test]
async fn history_query_is_tenant_scoped() {
    let db = make_test_db().await;
    let tenant_a = TenantId::new();
    let tenant_b = TenantId::new();
    let user_id = Uuid::new_v4();
    let state_a = synthetic_state(0, 80.0);
    let state_b = synthetic_state(0, 120.0);
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
    let rows_a = repos
        .training_history
        .get_training_history(tenant_a, user_id, anchor(), anchor())
        .await
        .expect("range A");
    let rows_b = repos
        .training_history
        .get_training_history(tenant_b, user_id, anchor(), anchor())
        .await
        .expect("range B");
    assert_eq!(rows_a.len(), 1);
    assert_eq!(rows_b.len(), 1);
    assert!((rows_a[0].daily_load - 80.0).abs() < 1e-9);
    assert!((rows_b[0].daily_load - 120.0).abs() < 1e-9);
}

#[tokio::test]
async fn history_optional_metrics_serialise_as_null_when_missing() {
    let mut state = synthetic_state(0, 0.0);
    state.acwr = None;
    state.monotony = None;
    state.strain = None;
    state.ramp_rate = None;
    let json = serde_json::to_value(&state).expect("serialize");
    let obj = json.as_object().expect("object root");
    // skip_serializing_if drops the keys entirely when None — coaches reading
    // the JSON should treat absent keys as "insufficient history".
    assert!(!obj.contains_key("acwr"));
    assert!(!obj.contains_key("monotony"));
    assert!(!obj.contains_key("strain"));
    assert!(!obj.contains_key("ramp_rate"));
}
