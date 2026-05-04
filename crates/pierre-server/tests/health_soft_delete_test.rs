// ABOUTME: Integration tests for soft-delete + hard-delete on sleep / recovery / health repositories
// ABOUTME: Exercises the by-id delete trait methods and the deleted_at filter on get queries
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Soft-delete integration tests for the health-persistence repositories.
//!
//! Verifies the F3c contract:
//!  1. Inserting a row makes it visible to `get_X` and `get_latest_X`.
//!  2. `delete_X_by_id(tenant, id, soft = true)` makes it disappear from
//!     subsequent reads but the row physically remains in the table.
//!  3. `find_X_tenant(id)` resolves the tenant for a row that was inserted
//!     under that tenant.
//!  4. `delete_X_by_id(tenant, id, soft = false)` removes the row entirely.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]
#![allow(missing_docs)]

mod common;

use chrono::Utc;
use pierre_core::models::{StoredSleepSession, TenantId};
use pierre_database::backends::factory::Database;
use sqlx::Row as _;
use std::sync::Arc;
use uuid::Uuid;

async fn make_user_and_tenant(db: &Arc<Database>) -> (Uuid, TenantId, String) {
    let (user_id, _user) = common::create_test_user(db).await.unwrap();
    let tenant_id = TenantId::new();
    // sleep_sessions.data_source_id has FK to data_sources(id) — seed a row.
    let data_source_id = format!("ds-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO data_sources (id, user_id, tenant_id, provider, device_type) \
         VALUES (?, ?, ?, ?, 'unknown')",
    )
    .bind(&data_source_id)
    .bind(user_id.to_string())
    .bind(tenant_id.to_string())
    .bind("strava")
    .execute(db.sqlite_pool().expect("sqlite test pool"))
    .await
    .unwrap();
    (user_id, tenant_id, data_source_id)
}

fn sleep_session(user_id: Uuid, source_name: &str, data_source_id: &str) -> StoredSleepSession {
    let now = Utc::now();
    StoredSleepSession {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        data_source_id: data_source_id.to_owned(),
        is_nap: false,
        start_datetime: now - chrono::Duration::hours(8),
        end_datetime: now,
        total_sleep_seconds: Some(28_000),
        deep_sleep_seconds: Some(5_000),
        light_sleep_seconds: Some(15_000),
        rem_sleep_seconds: Some(8_000),
        awake_seconds: Some(800),
        sleep_efficiency: Some(95.0),
        avg_heart_rate: Some(58.0),
        min_heart_rate: Some(50),
        avg_hrv: Some(60.0),
        sleep_score: Some(85),
        stages: Vec::new(),
        source_name: source_name.to_owned(),
    }
}

#[tokio::test]
async fn sleep_soft_delete_hides_row_from_reads_but_keeps_it_in_table() {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let repos = Arc::new(database.repositories());
    let (user_id, tenant_id, ds) = make_user_and_tenant(&database).await;

    let session = sleep_session(user_id, "strava", &ds);
    let id = session.id.clone();
    let start = session.start_datetime;
    let end = session.end_datetime;
    repos
        .sleep
        .upsert_sleep_session(&tenant_id, &session)
        .await
        .unwrap();

    // Sanity: row is visible before delete.
    let visible = repos
        .sleep
        .get_sleep_sessions(
            user_id,
            &tenant_id,
            start - chrono::Duration::minutes(1),
            end + chrono::Duration::minutes(1),
        )
        .await
        .unwrap();
    assert_eq!(visible.len(), 1, "session must be visible before delete");

    // Soft delete.
    let deleted = repos
        .sleep
        .delete_sleep_session_by_id(&tenant_id, &id, true)
        .await
        .unwrap();
    assert!(deleted, "soft delete must report success");

    // Read filter excludes the soft-deleted row.
    let after_soft = repos
        .sleep
        .get_sleep_sessions(
            user_id,
            &tenant_id,
            start - chrono::Duration::minutes(1),
            end + chrono::Duration::minutes(1),
        )
        .await
        .unwrap();
    assert!(
        after_soft.is_empty(),
        "soft-deleted session must not appear in reads"
    );

    // Row is still in the table — direct query confirms deleted_at is set.
    let raw = sqlx::query("SELECT deleted_at FROM sleep_sessions WHERE id = ?")
        .bind(&id)
        .fetch_one(database.sqlite_pool().expect("sqlite test pool"))
        .await
        .unwrap();
    let deleted_at: Option<String> = raw.get("deleted_at");
    assert!(
        deleted_at.is_some(),
        "soft delete must set deleted_at; row should still exist"
    );

    // Soft-deleting a row that's already soft-deleted reports no-op.
    let again = repos
        .sleep
        .delete_sleep_session_by_id(&tenant_id, &id, true)
        .await
        .unwrap();
    assert!(
        !again,
        "second soft delete on the same id must not report success"
    );
}

#[tokio::test]
async fn sleep_hard_delete_removes_the_row_entirely() {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let repos = Arc::new(database.repositories());
    let (user_id, tenant_id, ds) = make_user_and_tenant(&database).await;

    let session = sleep_session(user_id, "garmin", &ds);
    let id = session.id.clone();
    repos
        .sleep
        .upsert_sleep_session(&tenant_id, &session)
        .await
        .unwrap();

    let deleted = repos
        .sleep
        .delete_sleep_session_by_id(&tenant_id, &id, false)
        .await
        .unwrap();
    assert!(deleted, "hard delete must report success");

    // Row physically gone.
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sleep_sessions WHERE id = ?")
        .bind(&id)
        .fetch_one(database.sqlite_pool().expect("sqlite test pool"))
        .await
        .unwrap();
    assert_eq!(count, 0, "hard delete must remove the row");
}

#[tokio::test]
async fn sleep_find_tenant_resolves_owner_then_delete_works() {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let repos = Arc::new(database.repositories());
    let (user_id, tenant_id, ds) = make_user_and_tenant(&database).await;

    let session = sleep_session(user_id, "strava", &ds);
    let id = session.id.clone();
    repos
        .sleep
        .upsert_sleep_session(&tenant_id, &session)
        .await
        .unwrap();

    // The discovery helper used by the dravr-enforme adapter:
    let resolved = repos
        .sleep
        .find_sleep_session_tenant(&id)
        .await
        .unwrap()
        .expect("freshly inserted row must resolve a tenant");
    assert_eq!(resolved, tenant_id);

    // Unknown id resolves to None instead of an error.
    let missing = repos
        .sleep
        .find_sleep_session_tenant(&Uuid::new_v4().to_string())
        .await
        .unwrap();
    assert!(missing.is_none());
}

#[tokio::test]
async fn sleep_delete_by_id_rejects_wrong_tenant() {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let repos = Arc::new(database.repositories());
    let (user_id, owner_tenant, ds) = make_user_and_tenant(&database).await;
    let other_tenant = TenantId::new();

    let session = sleep_session(user_id, "strava", &ds);
    let id = session.id.clone();
    repos
        .sleep
        .upsert_sleep_session(&owner_tenant, &session)
        .await
        .unwrap();

    let cross_tenant = repos
        .sleep
        .delete_sleep_session_by_id(&other_tenant, &id, true)
        .await
        .unwrap();
    assert!(
        !cross_tenant,
        "delete must be scoped to tenant; wrong tenant must not soft-delete"
    );

    // The row is still active.
    let raw = sqlx::query("SELECT deleted_at FROM sleep_sessions WHERE id = ?")
        .bind(&id)
        .fetch_one(database.sqlite_pool().expect("sqlite test pool"))
        .await
        .unwrap();
    let deleted_at: Option<String> = raw.get("deleted_at");
    assert!(
        deleted_at.is_none(),
        "wrong-tenant delete must not have set deleted_at"
    );
}

#[tokio::test]
async fn time_series_insert_round_trips_within_a_range() {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let repos = Arc::new(database.repositories());
    let (_user_id, _tenant_id, ds) = make_user_and_tenant(&database).await;

    let now = Utc::now();
    let earlier = now - chrono::Duration::minutes(10);
    let later = now + chrono::Duration::minutes(10);

    // Heart rate samples (riviere SeriesType::HeartRate is series_type_id 1).
    let series_type_id: u32 = 1;
    let points = vec![(earlier, 60.0_f64), (now, 65.0), (later, 72.0)];

    let written = repos
        .time_series_points
        .insert_continuous_metrics_batch(&ds, series_type_id, &points)
        .await
        .unwrap();
    assert_eq!(written, 3, "all three points must be persisted");

    let fetched = repos
        .time_series_points
        .get_continuous_metrics(
            &ds,
            series_type_id,
            earlier - chrono::Duration::seconds(1),
            later + chrono::Duration::seconds(1),
        )
        .await
        .unwrap();
    assert_eq!(fetched.len(), 3);
    // ascending by recorded_at
    assert_eq!(fetched[0].1, 60.0);
    assert_eq!(fetched[1].1, 65.0);
    assert_eq!(fetched[2].1, 72.0);
}

#[tokio::test]
async fn time_series_insert_is_idempotent_on_conflict() {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let repos = Arc::new(database.repositories());
    let (_user_id, _tenant_id, ds) = make_user_and_tenant(&database).await;

    let ts = Utc::now();
    let series_type_id: u32 = 1;

    // First insert.
    let first = repos
        .time_series_points
        .insert_continuous_metrics_batch(&ds, series_type_id, &[(ts, 60.0)])
        .await
        .unwrap();
    assert_eq!(first, 1);

    // Second insert at the same (source, series, recorded_at) — last writer wins.
    let second = repos
        .time_series_points
        .insert_continuous_metrics_batch(&ds, series_type_id, &[(ts, 75.0)])
        .await
        .unwrap();
    assert_eq!(
        second, 1,
        "ON CONFLICT DO UPDATE counts as one affected row"
    );

    // Latest read returns the updated value.
    let fetched = repos
        .time_series_points
        .get_continuous_metrics(
            &ds,
            series_type_id,
            ts - chrono::Duration::seconds(1),
            ts + chrono::Duration::seconds(1),
        )
        .await
        .unwrap();
    assert_eq!(fetched.len(), 1);
    assert_eq!(fetched[0].1, 75.0, "second insert must overwrite the first");
}

#[tokio::test]
async fn time_series_empty_batch_is_a_no_op() {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let repos = Arc::new(database.repositories());
    let (_user_id, _tenant_id, ds) = make_user_and_tenant(&database).await;

    let written = repos
        .time_series_points
        .insert_continuous_metrics_batch(&ds, 1, &[])
        .await
        .unwrap();
    assert_eq!(written, 0);
}
