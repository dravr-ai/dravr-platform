// ABOUTME: Soft- and hard-delete on the sleep / recovery / health repositories, and the wellness writer's write modes
// ABOUTME: T9 (a whole-day write clears a retracted reading, a soft-deleted row comes back with nothing stale), plus S5
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
//!
//! The write modes: Athlete Data Consolidation, S4 and S5 (carnet#1305).
//!
//! Every ingress writes a night, a day's recovery or body row through
//! `WellnessStore`, naming its mode. These tests read back what each mode
//! leaves:
//!
//! - **T9**: a whole-day write (a provider sync) clears a reading the source
//!   retracted — an `athlete_note` blanked on intervals.icu — and a write of
//!   any mode onto a soft-deleted row brings the row back with only what the
//!   write carries, never the readings it held before the delete.
//! - A night's `SpO2` reaches its column.
//! - DECISION 2's migration clears the scores Body Battery filled on stored
//!   Garmin days, and nothing else.
//! - **S5**: a cached activity synced earlier than the stored copy does not
//!   overwrite it, and is not counted as written.
//!
//! T3 (the WHOOP clear) is in `whoop_terms_ingestion_test`; T8's two halves
//! — the roster leaves push holders out, two partial pushes for one day keep
//! every reading — are `garmin_connect_routing_test` and
//! `garmin_push_ingest_test::dailies_hrv_and_body_pushes_merge_on_their_days_rows`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp
)]
#![allow(missing_docs)]

use crate::common;
use crate::db_fixtures;

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use db_fixtures::seed_user;
use dravr_riviere::{Aggregation, DataPoint, SeriesType, TimeRange};
use pierre_core::models::{
    ActivityBuilder, DataSource, DeviceType, SportType, StoredHealthMetrics, StoredRecoveryMetrics,
    StoredSleepSession, TenantId,
};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::RepositoryRegistry;
use pierre_services::wellness_store::{WellnessLanded, WellnessStore, WellnessWriteMode};
use pierre_test_support::db::create_test_db;

use std::sync::Arc;
use uuid::Uuid;

/// `sleep_sessions.data_source_id` references `data_sources(id)`, so a source row is seeded.
const INSERT_SOURCE: &str =
    "INSERT INTO data_sources (id, user_id, tenant_id, provider, device_type) \
     VALUES ($1, $2, $3, $4, 'unknown')";

async fn make_user_and_tenant(db: &Arc<Database>) -> (Uuid, TenantId, String) {
    let (user_id, _user) = common::create_test_user(db).await.unwrap();
    let tenant_id = TenantId::generate();
    // sleep_sessions.data_source_id has FK to data_sources(id) — seed a row.
    let data_source_id = format!("ds-{}", Uuid::new_v4());
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query(INSERT_SOURCE)
                .bind(&data_source_id)
                .bind(user_id.to_string())
                .bind(tenant_id.to_string())
                .bind("strava")
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            sqlx::query(INSERT_SOURCE)
                .bind(&data_source_id)
                .bind(user_id.to_string())
                .bind(tenant_id.to_string())
                .bind("strava")
                .execute(pg.pool())
                .await
                .unwrap();
        }
    }
    (user_id, tenant_id, data_source_id)
}

/// Whether the row still exists and carries a `deleted_at`, read straight
/// from the table on whichever backend the test database is. `None` means the
/// row is gone.
async fn deleted_at_is_set(db: &Database, id: &str) -> Option<bool> {
    const SQL: &str = "SELECT deleted_at IS NOT NULL FROM sleep_sessions WHERE id = $1";
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => sqlx::query_scalar(SQL)
            .bind(id)
            .fetch_optional(sqlite.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => sqlx::query_scalar(SQL)
            .bind(id)
            .fetch_optional(pg.pool())
            .await
            .unwrap(),
    }
}

fn sleep_session(user_id: Uuid, source_name: &str, data_source_id: &str) -> StoredSleepSession {
    let now = Utc::now();
    StoredSleepSession {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        data_source_id: data_source_id.to_owned(),
        is_nap: false,
        start_datetime: now - Duration::hours(8),
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
        avg_spo2: None,
        stages: Vec::new(),
        source_name: source_name.to_owned(),
    }
}

#[tokio::test]
async fn sleep_soft_delete_hides_row_from_reads_but_keeps_it_in_table() {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let repos = Arc::clone(database.repositories());
    let (user_id, tenant_id, ds) = make_user_and_tenant(&database).await;

    let session = sleep_session(user_id, "strava", &ds);
    let id = session.id.clone();
    let start = session.start_datetime;
    let end = session.end_datetime;
    repos
        .sleep
        .upsert_sleep_session(&tenant_id, &session, Utc::now())
        .await
        .unwrap();

    // Sanity: row is visible before delete.
    let visible = repos
        .sleep
        .get_sleep_sessions(
            user_id,
            &tenant_id,
            start - Duration::minutes(1),
            end + Duration::minutes(1),
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
            start - Duration::minutes(1),
            end + Duration::minutes(1),
        )
        .await
        .unwrap();
    assert!(
        after_soft.is_empty(),
        "soft-deleted session must not appear in reads"
    );

    // Row is still in the table — direct query confirms deleted_at is set.
    assert_eq!(
        deleted_at_is_set(&database, &id).await,
        Some(true),
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
    let repos = Arc::clone(database.repositories());
    let (user_id, tenant_id, ds) = make_user_and_tenant(&database).await;

    let session = sleep_session(user_id, "garmin", &ds);
    let id = session.id.clone();
    repos
        .sleep
        .upsert_sleep_session(&tenant_id, &session, Utc::now())
        .await
        .unwrap();

    let deleted = repos
        .sleep
        .delete_sleep_session_by_id(&tenant_id, &id, false)
        .await
        .unwrap();
    assert!(deleted, "hard delete must report success");

    // Row physically gone.
    assert_eq!(
        deleted_at_is_set(&database, &id).await,
        None,
        "hard delete must remove the row"
    );
}

#[tokio::test]
async fn sleep_find_tenant_resolves_owner_then_delete_works() {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let repos = Arc::clone(database.repositories());
    let (user_id, tenant_id, ds) = make_user_and_tenant(&database).await;

    let session = sleep_session(user_id, "strava", &ds);
    let id = session.id.clone();
    repos
        .sleep
        .upsert_sleep_session(&tenant_id, &session, Utc::now())
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
    let repos = Arc::clone(database.repositories());
    let (user_id, owner_tenant, ds) = make_user_and_tenant(&database).await;
    let other_tenant = TenantId::generate();

    let session = sleep_session(user_id, "strava", &ds);
    let id = session.id.clone();
    repos
        .sleep
        .upsert_sleep_session(&owner_tenant, &session, Utc::now())
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
    assert_eq!(
        deleted_at_is_set(&database, &id).await,
        Some(false),
        "wrong-tenant delete must not have set deleted_at"
    );
}

#[tokio::test]
async fn time_series_insert_round_trips_within_a_range() {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let repos = Arc::clone(database.repositories());
    let (_user_id, _tenant_id, ds) = make_user_and_tenant(&database).await;

    let now = Utc::now();
    let earlier = now - Duration::minutes(10);
    let later = now + Duration::minutes(10);

    // Heart rate samples, keyed by riviere's SeriesType catalog.
    let series_type = SeriesType::HeartRate.id();
    let points = vec![
        DataPoint::new(earlier, 60.0),
        DataPoint::new(now, 65.0),
        DataPoint::new(later, 72.0),
    ];

    repos
        .time_series_points
        .insert_batch(&ds, series_type, points)
        .await
        .unwrap();

    let result = repos
        .time_series_points
        .query(
            &ds,
            series_type,
            &TimeRange::new(earlier - Duration::seconds(1), later + Duration::seconds(1)),
        )
        .await
        .unwrap();
    assert_eq!(result.points.len(), 3);
    assert_eq!(result.total_count, 3);
    // ascending by recorded_at
    assert_eq!(result.points[0].value, 60.0);
    assert_eq!(result.points[1].value, 65.0);
    assert_eq!(result.points[2].value, 72.0);
}

#[tokio::test]
async fn time_series_insert_is_idempotent_on_conflict() {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let repos = Arc::clone(database.repositories());
    let (_user_id, _tenant_id, ds) = make_user_and_tenant(&database).await;

    let ts = Utc::now();
    let series_type = SeriesType::HeartRate.id();

    // First insert.
    repos
        .time_series_points
        .insert_batch(&ds, series_type, vec![DataPoint::new(ts, 60.0)])
        .await
        .unwrap();

    // Second insert at the same (source, series, recorded_at) — last writer wins.
    repos
        .time_series_points
        .insert_batch(&ds, series_type, vec![DataPoint::new(ts, 75.0)])
        .await
        .unwrap();

    // The read collapses to a single point carrying the updated value.
    let result = repos
        .time_series_points
        .query(
            &ds,
            series_type,
            &TimeRange::new(ts - Duration::seconds(1), ts + Duration::seconds(1)),
        )
        .await
        .unwrap();
    assert_eq!(result.points.len(), 1);
    assert_eq!(
        result.points[0].value, 75.0,
        "second insert must overwrite the first"
    );
}

#[tokio::test]
async fn time_series_empty_batch_is_a_no_op() {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let repos = Arc::clone(database.repositories());
    let (_user_id, _tenant_id, ds) = make_user_and_tenant(&database).await;

    repos
        .time_series_points
        .insert_batch(&ds, SeriesType::HeartRate.id(), vec![])
        .await
        .unwrap();

    let result = repos
        .time_series_points
        .query(
            &ds,
            SeriesType::HeartRate.id(),
            &TimeRange::new(
                Utc::now() - Duration::minutes(1),
                Utc::now() + Duration::minutes(1),
            ),
        )
        .await
        .unwrap();
    assert_eq!(result.points.len(), 0, "empty batch persists nothing");
}

#[tokio::test]
async fn time_series_aggregate_downsamples_into_windows() {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let repos = Arc::clone(database.repositories());
    let (_user_id, _tenant_id, ds) = make_user_and_tenant(&database).await;

    let start = Utc::now();
    let series_type = SeriesType::HeartRate.id();
    let points = vec![
        DataPoint::new(start + Duration::seconds(5), 60.0),
        DataPoint::new(start + Duration::seconds(25), 70.0),
        DataPoint::new(start + Duration::seconds(65), 100.0),
    ];
    repos
        .time_series_points
        .insert_batch(&ds, series_type, points)
        .await
        .unwrap();

    let windows = repos
        .time_series_points
        .aggregate(
            &ds,
            series_type,
            &TimeRange::new(start, start + Duration::seconds(120)),
            60,
            Aggregation::Avg,
        )
        .await
        .unwrap();

    // First 60s window averages 60 and 70 -> 65; the second holds the lone 100.
    assert_eq!(windows.len(), 2);
    assert_eq!(windows[0].value, 65.0);
    assert_eq!(windows[0].sample_count, 2);
    assert_eq!(windows[1].value, 100.0);
    assert_eq!(windows[1].sample_count, 1);
}

#[tokio::test]
async fn time_series_latest_returns_most_recent_point() {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let repos = Arc::clone(database.repositories());
    let (_user_id, _tenant_id, ds) = make_user_and_tenant(&database).await;

    let now = Utc::now();
    let series_type = SeriesType::HeartRate.id();

    assert!(
        repos
            .time_series_points
            .latest(&ds, series_type)
            .await
            .unwrap()
            .is_none(),
        "no points yet"
    );

    repos
        .time_series_points
        .insert_batch(
            &ds,
            series_type,
            vec![
                DataPoint::new(now - Duration::minutes(5), 55.0),
                DataPoint::new(now, 80.0),
            ],
        )
        .await
        .unwrap();

    let latest = repos
        .time_series_points
        .latest(&ds, series_type)
        .await
        .unwrap()
        .expect("a point exists");
    assert_eq!(latest.value, 80.0);
}

/// The `SQLite` and `PostgreSQL` DECISION 2 migrations under test.
const SQLITE_MIGRATION: &str =
    include_str!("../../../../migrations/20261031120000_body_battery_is_not_a_score.sql");
#[cfg(feature = "postgresql")]
const POSTGRES_MIGRATION: &str =
    include_str!("../../../../migrations_pg/20261031120000_body_battery_is_not_a_score.sql");

fn morning() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 21).unwrap()
}

fn night_start() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 20, 23, 0, 0).unwrap()
}

async fn data_source(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: TenantId,
    provider: &str,
) -> String {
    WellnessStore::new(repos)
        .record_data_source(
            &tenant,
            &DataSource {
                id: String::new(),
                user_id: user_id.to_string(),
                provider: provider.to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Unknown,
                original_source_name: None,
            },
        )
        .await
        .unwrap()
}

fn day(user_id: Uuid, data_source_id: &str, provider: &str) -> StoredRecoveryMetrics {
    StoredRecoveryMetrics {
        id: format!("{provider}-day-{}", Uuid::new_v4()),
        user_id: user_id.to_string(),
        data_source_id: data_source_id.to_owned(),
        date: morning(),
        recovery_score: None,
        readiness_score: None,
        hrv_ms: None,
        hrv_rmssd: None,
        resting_heart_rate: None,
        stress_score: None,
        body_battery: None,
        spo2: None,
        respiratory_rate: None,
        skin_temp_deviation: None,
        daily_strain: None,
        athlete_note: None,
        source_name: provider.to_owned(),
        recorded_at: night_start() + Duration::hours(8),
    }
}

fn body(user_id: Uuid, data_source_id: &str, provider: &str) -> StoredHealthMetrics {
    StoredHealthMetrics {
        id: String::new(),
        user_id: user_id.to_string(),
        data_source_id: data_source_id.to_owned(),
        date: morning(),
        weight_kg: None,
        body_fat_pct: None,
        muscle_mass_kg: None,
        bmi: None,
        bone_mass_kg: None,
        water_pct: None,
        systolic_bp: None,
        diastolic_bp: None,
        blood_glucose: None,
        vo2_max: None,
        source_name: provider.to_owned(),
        recorded_at: night_start() + Duration::hours(8),
    }
}

fn night(user_id: Uuid, data_source_id: &str, provider: &str) -> StoredSleepSession {
    StoredSleepSession {
        id: format!("{provider}-night-{}", Uuid::new_v4()),
        user_id: user_id.to_string(),
        data_source_id: data_source_id.to_owned(),
        is_nap: false,
        start_datetime: night_start(),
        end_datetime: night_start() + Duration::hours(8),
        total_sleep_seconds: Some(26_000),
        deep_sleep_seconds: None,
        light_sleep_seconds: None,
        rem_sleep_seconds: None,
        awake_seconds: Some(2_000),
        sleep_efficiency: None,
        avg_heart_rate: Some(51.0),
        min_heart_rate: None,
        avg_hrv: None,
        sleep_score: None,
        avg_spo2: None,
        stages: Vec::new(),
        source_name: provider.to_owned(),
    }
}

async fn days(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: TenantId,
) -> Vec<StoredRecoveryMetrics> {
    repos
        .recovery
        .get_recovery_metrics(
            user_id,
            &tenant,
            night_start() - Duration::days(1),
            night_start() + Duration::days(2),
        )
        .await
        .unwrap()
}

async fn bodies(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: TenantId,
) -> Vec<StoredHealthMetrics> {
    repos
        .health_snapshots
        .get_health_snapshots(
            user_id,
            &tenant,
            night_start() - Duration::days(1),
            night_start() + Duration::days(2),
        )
        .await
        .unwrap()
}

async fn nights(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: TenantId,
) -> Vec<StoredSleepSession> {
    repos
        .sleep
        .get_sleep_sessions(
            user_id,
            &tenant,
            night_start() - Duration::days(1),
            night_start() + Duration::days(1),
        )
        .await
        .unwrap()
}

async fn setup() -> (Database, Arc<RepositoryRegistry>, Uuid, TenantId) {
    let db = create_test_db().await.unwrap();
    let repos = Arc::clone(db.repositories());
    let (user_id, tenant) = seed_user(&db).await;
    (db, repos, user_id, tenant)
}

/// T9, first half: a provider sync reads the whole day, so a reading the
/// athlete retracted on the provider is cleared from the stored row.
#[tokio::test]
async fn a_whole_day_write_clears_a_retracted_athlete_note() {
    let (_db, repos, user_id, tenant) = setup().await;
    let source = data_source(&repos, user_id, tenant, "intervals_icu").await;
    let store = WellnessStore::new(&repos);

    let noted = StoredRecoveryMetrics {
        athlete_note: Some("legs heavy after the long run".to_owned()),
        resting_heart_rate: Some(49),
        ..day(user_id, &source, "intervals_icu")
    };
    let mut writes = store.writes("recovery");
    writes
        .recovery(tenant, &noted, WellnessWriteMode::FullDay, Utc::now())
        .await
        .unwrap();
    let retracted = StoredRecoveryMetrics {
        athlete_note: None,
        ..noted.clone()
    };
    writes
        .recovery(tenant, &retracted, WellnessWriteMode::FullDay, Utc::now())
        .await
        .unwrap();

    let read = days(&repos, user_id, tenant).await;
    assert_eq!(read.len(), 1, "one day, one row: {read:?}");
    assert_eq!(
        read[0].athlete_note, None,
        "the note the athlete blanked on the provider is gone"
    );
    assert_eq!(
        read[0].resting_heart_rate,
        Some(49),
        "the readings the day still carries stay"
    );
}

/// T9, second half: a write of either mode onto a soft-deleted day brings
/// it back with only what the write carries. Before, neither write cleared
/// `deleted_at`, so a day the provider re-sent stayed invisible.
#[tokio::test]
async fn a_write_onto_a_soft_deleted_day_brings_back_nothing_from_before_the_delete() {
    let (_db, repos, user_id, tenant) = setup().await;
    let source = data_source(&repos, user_id, tenant, "garmin").await;
    let store = WellnessStore::new(&repos);
    let mut writes = store.writes("recovery");

    for mode in [
        WellnessWriteMode::PartialSummary,
        WellnessWriteMode::FullDay,
    ] {
        let before = StoredRecoveryMetrics {
            resting_heart_rate: Some(52),
            hrv_rmssd: Some(58.0),
            hrv_ms: Some(58.0),
            stress_score: Some(31),
            ..day(user_id, &source, "garmin")
        };
        let id = repos
            .recovery
            .upsert_recovery_metrics(&tenant, &before, WellnessWriteMode::FullDay, Utc::now())
            .await
            .unwrap();
        assert!(
            store.delete_recovery_metric(&id, true).await.unwrap(),
            "the day is soft-deleted"
        );
        assert!(
            days(&repos, user_id, tenant).await.is_empty(),
            "a soft-deleted day is not read"
        );

        let after = StoredRecoveryMetrics {
            body_battery: Some(73),
            ..day(user_id, &source, "garmin")
        };
        writes
            .recovery(tenant, &after, mode, Utc::now())
            .await
            .unwrap();

        let read = days(&repos, user_id, tenant).await;
        assert_eq!(read.len(), 1, "{mode:?}: the day is back: {read:?}");
        assert_eq!(
            read[0].body_battery,
            Some(73),
            "{mode:?}: the new reading lands"
        );
        assert_eq!(
            (
                read[0].resting_heart_rate,
                read[0].hrv_rmssd,
                read[0].stress_score
            ),
            (None, None, None),
            "{mode:?}: nothing stored before the delete comes back"
        );
        repos
            .recovery
            .delete_recovery_metric_by_id(&tenant, &read[0].id, false)
            .await
            .unwrap();
    }

    // The same for a body row and a night.
    let weighed = StoredHealthMetrics {
        weight_kg: Some(71.0),
        body_fat_pct: Some(14.0),
        ..body(user_id, &source, "garmin")
    };
    let id = repos
        .health_snapshots
        .upsert_health_snapshot(&tenant, &weighed, WellnessWriteMode::FullDay, Utc::now())
        .await
        .unwrap();
    assert!(
        store.delete_health_snapshot(&id, true).await.unwrap(),
        "the stored body row is found and soft-deleted"
    );
    let vo2 = StoredHealthMetrics {
        vo2_max: Some(54.0),
        ..body(user_id, &source, "garmin")
    };
    writes
        .body(tenant, &vo2, WellnessWriteMode::PartialSummary, Utc::now())
        .await
        .unwrap();
    let read = bodies(&repos, user_id, tenant).await;
    assert_eq!(read.len(), 1, "the body day is back: {read:?}");
    assert_eq!(
        (read[0].vo2_max, read[0].weight_kg, read[0].body_fat_pct),
        (Some(54.0), None, None),
        "only the new measurement stands on the body day"
    );

    let first = night(user_id, &source, "garmin");
    writes.sleep(tenant, &first, Utc::now()).await.unwrap();
    assert!(
        store.delete_sleep_session(&first.id, true).await.unwrap(),
        "the stored night is found and soft-deleted"
    );
    assert!(
        nights(&repos, user_id, tenant).await.is_empty(),
        "a soft-deleted night reads as gone"
    );
    let resent = StoredSleepSession {
        total_sleep_seconds: Some(25_000),
        avg_heart_rate: None,
        ..first.clone()
    };
    assert_eq!(
        writes.sleep(tenant, &resent, Utc::now()).await.unwrap(),
        WellnessLanded::Written,
        "a night resent onto its soft-deleted row is written"
    );
    let read = nights(&repos, user_id, tenant).await;
    assert_eq!(read.len(), 1, "the night is back: {read:?}");
    assert_eq!(
        (read[0].total_sleep_seconds, read[0].avg_heart_rate),
        (Some(25_000), None),
        "the night is the re-sent one, whole"
    );
}

/// A night's average `SpO2` reaches `sleep_sessions.avg_spo2`; the write
/// used to drop it.
#[tokio::test]
async fn a_nights_spo2_is_stored() {
    let (_db, repos, user_id, tenant) = setup().await;
    let source = data_source(&repos, user_id, tenant, "garmin").await;
    let with_spo2 = StoredSleepSession {
        avg_spo2: Some(95.5),
        ..night(user_id, &source, "garmin")
    };
    WellnessStore::new(&repos)
        .writes("sleep")
        .sleep(tenant, &with_spo2, Utc::now())
        .await
        .unwrap();
    let read = nights(&repos, user_id, tenant).await;
    assert_eq!(read.len(), 1, "one night is stored: {read:?}");
    assert_eq!(read[0].avg_spo2, Some(95.5), "the night's SpO2 is kept");
}

/// A night received earlier than the stored one does not replace it.
#[tokio::test]
async fn an_older_night_leaves_the_later_one_standing() {
    let (_db, repos, user_id, tenant) = setup().await;
    let source = data_source(&repos, user_id, tenant, "garmin").await;
    let mut writes = WellnessStore::new(&repos).writes("sleep");
    let later = Utc::now();
    let newer = StoredSleepSession {
        total_sleep_seconds: Some(27_000),
        ..night(user_id, &source, "garmin")
    };
    let older = StoredSleepSession {
        total_sleep_seconds: Some(20_000),
        ..newer.clone()
    };
    assert_eq!(
        writes.sleep(tenant, &newer, later).await.unwrap(),
        WellnessLanded::Written,
        "the first night received is written"
    );
    assert_eq!(
        writes
            .sleep(tenant, &older, later - Duration::minutes(5))
            .await
            .unwrap(),
        WellnessLanded::Superseded,
        "the older receipt is refused"
    );
    let read = nights(&repos, user_id, tenant).await;
    assert_eq!(
        read[0].total_sleep_seconds,
        Some(27_000),
        "the later night stands"
    );
}

/// S5: the one `cached_activities` writer keeps the latest receipt. A copy
/// synced earlier than the stored one — a push batch retried after a later
/// push of the same activity — neither overwrites it nor counts as written.
#[tokio::test]
async fn an_activity_synced_earlier_than_the_stored_copy_leaves_it_standing() {
    let (_db, repos, user_id, tenant) = setup().await;
    let start = Utc.with_ymd_and_hms(2026, 10, 20, 7, 0, 0).unwrap();
    let ride = |name: &str| {
        ActivityBuilder::new("pushed-1", name, SportType::Ride, start, 3_600, "garmin")
            .distance_meters(30_000.0)
            .build()
    };
    let cache = &repos.activity_cache;
    let later = Utc::now();
    let written = cache
        .upsert_activities_synced_at(user_id, &tenant, "garmin", &[ride("Final title")], later)
        .await
        .unwrap();
    assert_eq!(written, 1, "the later copy lands");
    let written = cache
        .upsert_activities_synced_at(
            user_id,
            &tenant,
            "garmin",
            &[ride("Tentative title")],
            later - Duration::minutes(10),
        )
        .await
        .unwrap();
    assert_eq!(
        written, 0,
        "a copy synced earlier is not written, so not counted"
    );
    let stored = cache
        .get_cached_activity(user_id, &tenant, "garmin", "pushed-1")
        .await
        .unwrap()
        .expect("the row is stored");
    assert_eq!(stored.name(), "Final title", "the later copy stands");

    let amended = ride("Amended title");
    assert!(
        cache
            .amend_activity(user_id, &tenant, "garmin", &amended)
            .await
            .unwrap(),
        "an amend rewrites the standing row's content"
    );
    let stored = cache
        .get_cached_activity(user_id, &tenant, "garmin", "pushed-1")
        .await
        .unwrap()
        .expect("the row is stored");
    assert_eq!(
        stored.name(),
        "Amended title",
        "the amend's title is the stored one"
    );
    assert_eq!(
        cache
            .latest_activity_sync(user_id, &tenant, "garmin")
            .await
            .unwrap()
            .map(|at| at.timestamp()),
        Some(later.timestamp()),
        "an amend leaves the row's receipt as it stands"
    );
}

/// Apply the DECISION 2 migration, whole, to whichever engine the test
/// database is.
async fn run_migration(db: &Database) {
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::raw_sql(SQLITE_MIGRATION)
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => {
            sqlx::raw_sql(POSTGRES_MIGRATION)
                .execute(postgres.pool())
                .await
                .unwrap();
        }
    }
}

/// DECISION 2: the migration clears the recovery and readiness scores Body
/// Battery filled on stored Garmin days, keeps Body Battery, and touches no
/// score that is not a copy of it, nor another provider's.
#[tokio::test]
async fn the_migration_clears_body_battery_scores_and_nothing_else() {
    let (db, repos, user_id, tenant) = setup().await;
    let garmin = data_source(&repos, user_id, tenant, "garmin").await;
    let strava = data_source(&repos, user_id, tenant, "strava").await;
    let copied = StoredRecoveryMetrics {
        body_battery: Some(64),
        recovery_score: Some(64),
        readiness_score: Some(64),
        resting_heart_rate: Some(48),
        ..day(user_id, &garmin, "garmin")
    };
    let not_a_copy = StoredRecoveryMetrics {
        date: morning() - Duration::days(1),
        body_battery: Some(70),
        recovery_score: Some(55),
        ..day(user_id, &garmin, "garmin")
    };
    let other_provider = StoredRecoveryMetrics {
        recovery_score: Some(64),
        readiness_score: Some(64),
        body_battery: Some(64),
        ..day(user_id, &strava, "strava")
    };
    for row in [&copied, &not_a_copy, &other_provider] {
        repos
            .recovery
            .upsert_recovery_metrics(&tenant, row, WellnessWriteMode::FullDay, Utc::now())
            .await
            .unwrap();
    }

    run_migration(&db).await;

    let read = days(&repos, user_id, tenant).await;
    let find = |provider: &str, date: NaiveDate| {
        read.iter()
            .find(|d| d.source_name == provider && d.date == date)
            .unwrap_or_else(|| panic!("no {provider} day on {date}"))
    };
    let garmin_day = find("garmin", morning());
    assert_eq!(
        (garmin_day.recovery_score, garmin_day.readiness_score),
        (None, None),
        "the scores Body Battery filled are cleared"
    );
    assert_eq!(
        (garmin_day.body_battery, garmin_day.resting_heart_rate),
        (Some(64), Some(48)),
        "Body Battery and the day's measurements stay"
    );
    assert_eq!(
        find("garmin", morning() - Duration::days(1)).recovery_score,
        Some(55),
        "a score that is not a copy of Body Battery is left alone"
    );
    let strava_day = find("strava", morning());
    assert_eq!(
        (strava_day.recovery_score, strava_day.readiness_score),
        (Some(64), Some(64)),
        "another provider's scores are not Garmin's to clear"
    );
}
