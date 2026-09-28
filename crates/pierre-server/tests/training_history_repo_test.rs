// ABOUTME: Endurance Phase 2 — TrainingHistoryRepository multi-tenant isolation + round-trip + range queries
// ABOUTME: Also pins that a row stored without form_ctl (same-day TSB, pre-carnet#601) is never served
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{Duration, NaiveDate};
use pierre_core::models::{DailyTrainingState, TenantId};
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db_with_key;
use pierre_database::DatabaseProvider;
use std::slice;
use uuid::Uuid;

/// What an older binary's upsert writes: every metric but `form_ctl`, with
/// the same-day TSB it computed (CTL 60, ATL 45 read as +15, fresh).
async fn insert_as_an_older_binary(
    db: &Database,
    tenant_id: TenantId,
    user_id: Uuid,
    date: NaiveDate,
) {
    const SQL: &str =
        "INSERT INTO training_history (tenant_id, user_id, date, ctl, atl, tsb, daily_load) \
                       VALUES ($1, $2, $3, 60.0, 45.0, 15.0, 0.0)";
    let inserted = match db {
        Database::SQLite(inner) => sqlx::query(SQL)
            .bind(tenant_id)
            .bind(user_id.to_string())
            .bind(date)
            .execute(inner.pool())
            .await
            .unwrap()
            .rows_affected(),
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(inner) => sqlx::query(SQL)
            .bind(tenant_id)
            .bind(user_id)
            .bind(date)
            .execute(inner.pool())
            .await
            .unwrap()
            .rows_affected(),
    };
    assert_eq!(inserted, 1, "the stale row is in the table");
}

async fn make_test_db() -> Database {
    let encryption_key = b"test_encryption_key_32_bytes_long".to_vec();
    let db = create_test_db_with_key(encryption_key)
        .await
        .expect("create db");
    db.migrate().await.expect("migrate");
    db
}

fn anchor() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()
}

/// A day whose CTL rose by one overnight: form (`tsb`) is yesterday's
/// balance, read against yesterday's CTL (`form_ctl = ctl - 1`).
fn synthetic_state(day_offset: i64, ctl: f64, atl: f64, daily_load: f64) -> DailyTrainingState {
    DailyTrainingState {
        date: anchor() + Duration::days(day_offset),
        ctl,
        atl,
        tsb: ctl - atl,
        form_ctl: ctl - 1.0,
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
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4();
    let state = synthetic_state(0, 45.0, 50.0, 80.0);
    db.repositories()
        .training_history
        .upsert_training_history_batch(tenant_id, user_id, slice::from_ref(&state))
        .await
        .expect("upsert");
    let read = db
        .repositories()
        .training_history
        .get_training_history(
            tenant_id,
            user_id,
            anchor() - Duration::days(3650),
            anchor() + Duration::days(3650),
        )
        .await
        .expect("latest")
        .pop()
        .expect("row present");
    assert_eq!(read.date, state.date);
    assert!((read.ctl - 45.0).abs() < 1e-9);
    assert!((read.atl - 50.0).abs() < 1e-9);
    assert!((read.tsb - (-5.0)).abs() < 1e-9);
    assert!(
        (read.form_ctl - 44.0).abs() < 1e-9,
        "the CTL form is read against survives the round trip: {}",
        read.form_ctl
    );
    assert_eq!(read.acwr, Some(1.0));
}

/// A row without `form_ctl` carries the TSB the platform stored before form
/// followed the Coggan/TrainingPeaks convention, or one an older binary wrote
/// during a rollout. It is a different number with nothing to band it
/// against, so the read skips it while serving the current rows around it.
#[tokio::test]
async fn a_row_stored_without_form_ctl_is_never_served() {
    let db = make_test_db().await;
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4();

    insert_as_an_older_binary(&db, tenant_id, user_id, anchor()).await;
    let current = synthetic_state(1, 45.0, 50.0, 80.0);
    db.repositories()
        .training_history
        .upsert_training_history_batch(tenant_id, user_id, slice::from_ref(&current))
        .await
        .expect("upsert");

    let rows = db
        .repositories()
        .training_history
        .get_training_history(tenant_id, user_id, anchor(), anchor() + Duration::days(1))
        .await
        .expect("range");
    assert_eq!(
        rows.iter().map(|r| r.date).collect::<Vec<_>>(),
        vec![current.date],
        "only the row carrying form_ctl is served: {rows:?}"
    );

    // Recomputing the stale day replaces it, and it is served again.
    let recomputed = synthetic_state(0, 40.0, 45.0, 70.0);
    db.repositories()
        .training_history
        .upsert_training_history_batch(tenant_id, user_id, slice::from_ref(&recomputed))
        .await
        .expect("recompute");
    let rows = db
        .repositories()
        .training_history
        .get_training_history(tenant_id, user_id, anchor(), anchor())
        .await
        .expect("range");
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!((rows[0].tsb - (-5.0)).abs() < 1e-9, "{rows:?}");
    assert!((rows[0].form_ctl - 39.0).abs() < 1e-9, "{rows:?}");
}

#[tokio::test]
async fn upsert_overwrites_existing_day() {
    let db = make_test_db().await;
    let tenant_id = TenantId::generate();
    let user_id = Uuid::new_v4();
    let v1 = synthetic_state(0, 40.0, 45.0, 70.0);
    let v2 = synthetic_state(0, 50.0, 55.0, 90.0);
    let repos = db.repositories();
    repos
        .training_history
        .upsert_training_history_batch(tenant_id, user_id, slice::from_ref(&v1))
        .await
        .expect("upsert v1");
    repos
        .training_history
        .upsert_training_history_batch(tenant_id, user_id, slice::from_ref(&v2))
        .await
        .expect("upsert v2");
    let read = repos
        .training_history
        .get_training_history(
            tenant_id,
            user_id,
            anchor() - Duration::days(3650),
            anchor() + Duration::days(3650),
        )
        .await
        .expect("latest")
        .pop()
        .expect("row");
    assert!((read.ctl - 50.0).abs() < 1e-9);
    assert!((read.daily_load - 90.0).abs() < 1e-9);
}

#[tokio::test]
async fn batch_upsert_and_range_query_return_chronological_order() {
    let db = make_test_db().await;
    let tenant_id = TenantId::generate();
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
    let tenant_id = TenantId::generate();
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
    let tenant_a = TenantId::generate();
    let tenant_b = TenantId::generate();
    let user_id = Uuid::new_v4();
    let state_a = synthetic_state(0, 30.0, 35.0, 60.0);
    let state_b = synthetic_state(0, 70.0, 65.0, 120.0);
    let repos = db.repositories();
    repos
        .training_history
        .upsert_training_history_batch(tenant_a, user_id, slice::from_ref(&state_a))
        .await
        .expect("upsert A");
    repos
        .training_history
        .upsert_training_history_batch(tenant_b, user_id, slice::from_ref(&state_b))
        .await
        .expect("upsert B");
    let read_a = repos
        .training_history
        .get_training_history(
            tenant_a,
            user_id,
            anchor() - Duration::days(3650),
            anchor() + Duration::days(3650),
        )
        .await
        .expect("read A")
        .pop()
        .expect("row A");
    let read_b = repos
        .training_history
        .get_training_history(
            tenant_b,
            user_id,
            anchor() - Duration::days(3650),
            anchor() + Duration::days(3650),
        )
        .await
        .expect("read B")
        .pop()
        .expect("row B");
    assert!((read_a.ctl - 30.0).abs() < 1e-9);
    assert!((read_b.ctl - 70.0).abs() < 1e-9);
    // Cross-tenant read with the wrong user must miss.
    let other_user = Uuid::new_v4();
    let miss = repos
        .training_history
        .get_training_history(
            tenant_a,
            other_user,
            anchor() - Duration::days(3650),
            anchor() + Duration::days(3650),
        )
        .await
        .expect("read")
        .pop()
        .map(|r| r.ctl);
    assert!(miss.is_none());
}

#[tokio::test]
async fn empty_history_returns_no_rows() {
    let db = make_test_db().await;
    let latest = db
        .repositories()
        .training_history
        .get_training_history(
            TenantId::generate(),
            Uuid::new_v4(),
            anchor() - Duration::days(3650),
            anchor() + Duration::days(3650),
        )
        .await
        .expect("read");
    assert!(latest.is_empty());
}
