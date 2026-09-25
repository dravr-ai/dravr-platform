// ABOUTME: carnet#539 — WHOOP's own scores never reach storage, and WHOOP sleep efficiency is Dravr's own figure
// ABOUTME: Sync, workouts, and the migration over rows already stored, each asserted on the values read back

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! WHOOP's API Terms (effective 2026-10-06, §4) leave WHOOP's own
//! calculations — recovery %, strain, sleep performance, WHOOP's sleep
//! efficiency — for WHOOP alone to authorize storing. Health sync used to
//! write every one of them. These tests hold the three ways WHOOP data
//! reaches the database to that: a synced night and day keep their
//! measurements and lose WHOOP's scores, a WHOOP workout reaches the activity
//! cache without its strain, and the migration clears the copies already
//! stored. Other providers' scores are asserted untouched each time, so a
//! policy that dropped everything would fail too.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
#![cfg(all(feature = "health-sync", feature = "provider-whoop"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

#[path = "helpers/db_fixtures.rs"]
mod db_fixtures;

use std::borrow::Cow;
use std::sync::{Arc, Once};

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use db_fixtures::seed_user;
use pierre_config::environment::HttpClientConfig;
use pierre_core::models::{
    ActivityBuilder, DataSource, DeviceType, SportType, StoredRecoveryMetrics, StoredSleepSession,
    TenantId, UserOAuthToken,
};
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db_with_key;
use pierre_database::RepositoryRegistry;
use pierre_enforme::traits::recovery_store::RecoveryStore;
use pierre_enforme::traits::sleep_store::SleepStore;
use pierre_mcp_server::constants::init_server_config;
use pierre_mcp_server::utils::http_client::initialize_http_clients;
use pierre_providers::core::{FitnessProvider, OAuth2Credentials, ProviderConfig};
use pierre_providers::whoop_provider::WhoopProvider;
use pierre_services::health_sync::PierreSyncStorage;
use pierre_services::whoop_terms::{in_house_sleep_efficiency, sleep_session_to_store};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use uuid::Uuid;

/// The night every fixture sleeps: 22:00 to 06:00 UTC, 28 800 s in bed.
const NIGHT_SECONDS: i64 = 28_800;

/// Time awake inside that night.
const AWAKE_SECONDS: u32 = 1_800;

/// Dravr's efficiency for it: (28 800 − 1 800) / 28 800 × 100.
const DRAVR_EFFICIENCY: f64 = 93.75;

/// WHOOP's own efficiency for the same night, which must never be stored.
const WHOOP_EFFICIENCY: f64 = 97.0;

/// WHOOP's sleep performance score for it, which must never be stored.
const WHOOP_SLEEP_PERFORMANCE: u32 = 88;

/// The `SQLite` and `PostgreSQL` migrations under test.
const SQLITE_MIGRATION: &str =
    include_str!("../../../migrations/20260924183901_whoop_proprietary_scores_withheld.sql");
#[cfg(feature = "postgresql")]
const POSTGRES_MIGRATION: &str =
    include_str!("../../../migrations_pg/20260924183901_whoop_proprietary_scores_withheld.sql");

static HTTP_INIT: Once = Once::new();

/// A migrated test database whose key is a valid AES-256 key, so the token
/// rows the sync adapter reads can be written.
async fn create_test_db() -> Database {
    create_test_db_with_key(b"whoop-terms-test-key-32-bytes!!!".to_vec())
        .await
        .unwrap()
}

fn night_start() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 20, 22, 0, 0).unwrap()
}

fn morning() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 21).unwrap()
}

/// A token row for `provider`: the sync adapter recovers the tenant from it.
async fn connect(repos: &RepositoryRegistry, user_id: Uuid, tenant: TenantId, provider: &str) {
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id: tenant.to_string(),
            provider: provider.to_owned(),
            access_token: format!("{provider}-access-do-not-log"),
            refresh_token: None,
            token_type: "Bearer".to_owned(),
            expires_at: Some(Utc::now() + Duration::hours(1)),
            scope: Some("read".to_owned()),
            provider_user_id: None,
            oauth_app_client_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
}

/// The persisted data source a sync stamps on each of `provider`'s records.
async fn data_source(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: TenantId,
    provider: &str,
) -> String {
    repos
        .data_sources
        .upsert_data_source(
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

/// A night as WHOOP's sync adapter hands it over: measurements plus WHOOP's
/// own sleep performance and sleep efficiency.
fn whoop_night(user_id: Uuid, data_source_id: &str, start: DateTime<Utc>) -> StoredSleepSession {
    StoredSleepSession {
        id: format!("whoop-sleep-{}", Uuid::new_v4()),
        user_id: user_id.to_string(),
        data_source_id: data_source_id.to_owned(),
        is_nap: false,
        start_datetime: start,
        end_datetime: start + Duration::seconds(NIGHT_SECONDS),
        total_sleep_seconds: Some(25_200),
        deep_sleep_seconds: Some(5_400),
        light_sleep_seconds: Some(12_600),
        rem_sleep_seconds: Some(7_200),
        awake_seconds: Some(AWAKE_SECONDS),
        sleep_efficiency: Some(WHOOP_EFFICIENCY),
        avg_heart_rate: Some(52.5),
        min_heart_rate: Some(46),
        avg_hrv: Some(71.0),
        sleep_score: Some(WHOOP_SLEEP_PERFORMANCE),
        stages: Vec::new(),
        source_name: "whoop".to_owned(),
    }
}

/// The same night from Garmin, with Garmin's own score and efficiency.
fn garmin_night(user_id: Uuid, data_source_id: &str) -> StoredSleepSession {
    StoredSleepSession {
        id: format!("garmin-sleep-{}", Uuid::new_v4()),
        sleep_score: Some(81),
        sleep_efficiency: Some(90.0),
        source_name: "garmin".to_owned(),
        ..whoop_night(user_id, data_source_id, night_start())
    }
}

/// A morning as WHOOP's sync adapter hands it over: measurements plus
/// WHOOP's recovery % and day strain.
fn whoop_day(user_id: Uuid, data_source_id: &str) -> StoredRecoveryMetrics {
    StoredRecoveryMetrics {
        id: format!("whoop-cycle-{}", Uuid::new_v4()),
        user_id: user_id.to_string(),
        data_source_id: data_source_id.to_owned(),
        date: morning(),
        recovery_score: Some(34),
        readiness_score: None,
        hrv_ms: Some(61.5),
        hrv_rmssd: Some(61.5),
        resting_heart_rate: Some(52),
        stress_score: None,
        body_battery: None,
        spo2: Some(96.0),
        respiratory_rate: Some(15.5),
        skin_temp_deviation: Some(33.25),
        daily_strain: Some(14.2),
        athlete_note: None,
        source_name: "whoop".to_owned(),
        recorded_at: night_start() + Duration::seconds(NIGHT_SECONDS),
    }
}

/// The same morning from Garmin, with Garmin's own composite scores.
fn garmin_day(user_id: Uuid, data_source_id: &str) -> StoredRecoveryMetrics {
    StoredRecoveryMetrics {
        id: format!("garmin-day-{}", Uuid::new_v4()),
        recovery_score: Some(40),
        body_battery: Some(60),
        stress_score: Some(25),
        daily_strain: None,
        source_name: "garmin".to_owned(),
        ..whoop_day(user_id, data_source_id)
    }
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
            night_start() - Duration::days(3),
            night_start() + Duration::days(1),
        )
        .await
        .unwrap()
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
            night_start() - Duration::days(3),
            night_start() + Duration::days(2),
        )
        .await
        .unwrap()
}

fn from<'a, T>(rows: &'a [T], source: &str, name: impl Fn(&T) -> &str) -> &'a T {
    rows.iter()
        .find(|row| name(row) == source)
        .unwrap_or_else(|| panic!("no {source} row among {} read back", rows.len()))
}

/// Apply the migration under test, whole, to whichever engine the test database is.
async fn run_migration(db: &Database) {
    match db {
        Database::SQLite(sqlite) => {
            sqlx::raw_sql(SQLITE_MIGRATION)
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(postgres) => {
            sqlx::raw_sql(POSTGRES_MIGRATION)
                .execute(postgres.pool())
                .await
                .unwrap();
        }
    }
}

#[test]
fn dravr_sleep_efficiency_is_the_share_of_time_in_bed_not_spent_awake() {
    let start = night_start();
    let end = start + Duration::seconds(NIGHT_SECONDS);
    assert_eq!(
        in_house_sleep_efficiency(start, end, Some(AWAKE_SECONDS)),
        Some(DRAVR_EFFICIENCY)
    );
    assert_eq!(in_house_sleep_efficiency(start, end, Some(0)), Some(100.0));
    // Nothing to compute from: unknown awake time, an empty session, or more
    // awake time than the session holds.
    assert_eq!(in_house_sleep_efficiency(start, end, None), None);
    assert_eq!(in_house_sleep_efficiency(start, start, Some(0)), None);
    assert_eq!(in_house_sleep_efficiency(end, start, Some(0)), None);
    assert_eq!(in_house_sleep_efficiency(start, end, Some(30_000)), None);
}

#[test]
fn another_providers_night_passes_through_untouched() {
    let garmin = garmin_night(Uuid::new_v4(), "ds");
    let kept = sleep_session_to_store(&garmin);
    assert!(
        matches!(kept, Cow::Borrowed(_)),
        "a Garmin night is not copied"
    );
    assert_eq!(kept.sleep_score, Some(81));
    assert_eq!(kept.sleep_efficiency, Some(90.0));

    let whoop = whoop_night(Uuid::new_v4(), "ds", night_start());
    let kept = sleep_session_to_store(&whoop);
    assert_eq!(kept.sleep_score, None);
    assert_eq!(kept.sleep_efficiency, Some(DRAVR_EFFICIENCY));
}

#[tokio::test]
async fn a_synced_whoop_night_keeps_its_measurements_and_none_of_whoops_scores() {
    let db = create_test_db().await;
    let repos = Arc::new(db.repositories());
    let (user_id, tenant) = seed_user(&db).await;
    connect(&repos, user_id, tenant, "whoop").await;
    connect(&repos, user_id, tenant, "sciotte_garmin").await;
    let whoop_ds = data_source(&repos, user_id, tenant, "whoop").await;
    let garmin_ds = data_source(&repos, user_id, tenant, "garmin").await;
    let storage = PierreSyncStorage::new(&repos);

    let stored = storage
        .store_sleep_sessions(&[
            whoop_night(user_id, &whoop_ds, night_start()),
            garmin_night(user_id, &garmin_ds),
        ])
        .await
        .unwrap();
    assert_eq!(stored, 2);

    let read = nights(&repos, user_id, tenant).await;
    assert_eq!(read.len(), 2, "{read:?}");
    let whoop = from(&read, "whoop", |n| n.source_name.as_str());
    assert_eq!(
        whoop.sleep_score, None,
        "WHOOP's sleep performance score is never stored"
    );
    assert_eq!(
        whoop.sleep_efficiency,
        Some(DRAVR_EFFICIENCY),
        "WHOOP's own efficiency ({WHOOP_EFFICIENCY}) is replaced by Dravr's"
    );
    assert_eq!(whoop.total_sleep_seconds, Some(25_200));
    assert_eq!(whoop.awake_seconds, Some(AWAKE_SECONDS));
    assert_eq!(whoop.deep_sleep_seconds, Some(5_400));
    assert_eq!(whoop.light_sleep_seconds, Some(12_600));
    assert_eq!(whoop.rem_sleep_seconds, Some(7_200));
    assert_eq!(whoop.avg_hrv, Some(71.0));
    assert_eq!(whoop.min_heart_rate, Some(46));
    assert_eq!(whoop.avg_heart_rate, Some(52.5));

    let garmin = from(&read, "garmin", |n| n.source_name.as_str());
    assert_eq!(
        garmin.sleep_score,
        Some(81),
        "Garmin's score is not WHOOP's"
    );
    assert_eq!(garmin.sleep_efficiency, Some(90.0));
}

#[tokio::test]
async fn a_synced_whoop_day_keeps_its_measurements_and_drops_recovery_and_strain() {
    let db = create_test_db().await;
    let repos = Arc::new(db.repositories());
    let (user_id, tenant) = seed_user(&db).await;
    connect(&repos, user_id, tenant, "whoop").await;
    connect(&repos, user_id, tenant, "sciotte_garmin").await;
    let whoop_ds = data_source(&repos, user_id, tenant, "whoop").await;
    let garmin_ds = data_source(&repos, user_id, tenant, "garmin").await;
    let storage = PierreSyncStorage::new(&repos);

    let stored = storage
        .store_recovery_metrics(&[
            whoop_day(user_id, &whoop_ds),
            garmin_day(user_id, &garmin_ds),
        ])
        .await
        .unwrap();
    assert_eq!(stored, 2);

    let read = days(&repos, user_id, tenant).await;
    assert_eq!(read.len(), 2, "{read:?}");
    let whoop = from(&read, "whoop", |d| d.source_name.as_str());
    assert_eq!(
        whoop.recovery_score, None,
        "WHOOP's recovery % is never stored"
    );
    assert_eq!(
        whoop.daily_strain, None,
        "WHOOP's day strain is never stored"
    );
    assert_eq!(whoop.readiness_score, None);
    assert_eq!(whoop.stress_score, None);
    assert_eq!(whoop.body_battery, None);
    assert_eq!(whoop.hrv_rmssd, Some(61.5));
    assert_eq!(whoop.hrv_ms, Some(61.5));
    assert_eq!(whoop.resting_heart_rate, Some(52));
    assert_eq!(whoop.spo2, Some(96.0));
    assert_eq!(whoop.respiratory_rate, Some(15.5));
    assert_eq!(whoop.skin_temp_deviation, Some(33.25));

    let garmin = from(&read, "garmin", |d| d.source_name.as_str());
    assert_eq!(garmin.recovery_score, Some(40));
    assert_eq!(garmin.body_battery, Some(60));
    assert_eq!(garmin.stress_score, Some(25));
}

/// A sync still in flight when the athlete disconnects WHOOP hands over
/// records after the disconnect has deleted the grant and purged the rows.
/// Resolving the tenant from any other token the athlete holds would write
/// them straight back; they are refused instead. The rule is every
/// provider's: a record is written only under the tenant of the connection
/// that serves its own provider.
#[tokio::test]
async fn a_whoop_record_is_refused_once_the_athletes_whoop_grant_is_gone() {
    let db = create_test_db().await;
    let repos = Arc::new(db.repositories());
    let (user_id, tenant) = seed_user(&db).await;
    connect(&repos, user_id, tenant, "sciotte_garmin").await;
    let whoop_ds = data_source(&repos, user_id, tenant, "whoop").await;
    let intervals_ds = data_source(&repos, user_id, tenant, "intervals_icu").await;
    let storage = PierreSyncStorage::new(&repos);

    assert!(storage
        .store_sleep_sessions(&[whoop_night(user_id, &whoop_ds, night_start())])
        .await
        .is_err());
    assert!(storage
        .store_recovery_metrics(&[whoop_day(user_id, &whoop_ds)])
        .await
        .is_err());
    assert!(nights(&repos, user_id, tenant).await.is_empty());
    assert!(days(&repos, user_id, tenant).await.is_empty());

    // intervals.icu holds no token here either: its night is refused too,
    // rather than written under the athlete's Garmin connection.
    let intervals_night = StoredSleepSession {
        source_name: "intervals_icu".to_owned(),
        ..garmin_night(user_id, &intervals_ds)
    };
    assert!(storage
        .store_sleep_sessions(&[intervals_night])
        .await
        .is_err());
    assert!(nights(&repos, user_id, tenant).await.is_empty());

    // Garmin's own night resolves through its sciotte session.
    let garmin_ds = data_source(&repos, user_id, tenant, "garmin").await;
    let stored = storage
        .store_sleep_sessions(&[garmin_night(user_id, &garmin_ds)])
        .await
        .unwrap();
    assert_eq!(stored, 1);
    let read = nights(&repos, user_id, tenant).await;
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].source_name, "garmin");
}

/// Serve one HTTP response to the first request and hand back the base URL.
async fn serve_once(body: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let mut buf = vec![0_u8; 8192];
        let _ = stream.read(&mut buf).await;
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.shutdown().await;
    });
    base
}

#[tokio::test]
async fn a_whoop_workout_reaches_the_activity_model_without_its_strain() {
    HTTP_INIT.call_once(|| {
        let _ = init_server_config();
        initialize_http_clients(HttpClientConfig::default());
    });
    let workout = serde_json::json!({
        "id": "5b8e1c2a-0000-4000-8000-000000000539",
        "start": "2026-09-21T11:00:00.000Z",
        "end": "2026-09-21T12:00:00.000Z",
        "sport_id": 1,
        "score": {
            "strain": 12.3,
            "average_heart_rate": 142,
            "max_heart_rate": 171,
            "kilojoule": 2100.0,
            "distance_meter": 10000.0,
            "altitude_gain_meter": 85.0
        }
    });
    let base = serve_once(workout.to_string()).await;
    let provider = WhoopProvider::with_config(ProviderConfig {
        name: "whoop".to_owned(),
        auth_url: format!("{base}/oauth/oauth2/auth"),
        token_url: format!("{base}/oauth/oauth2/token"),
        api_base_url: base.clone(),
        revoke_url: None,
        default_scopes: Vec::new(),
    });
    provider
        .set_credentials(OAuth2Credentials {
            client_id: "whoop-test-client".to_owned(),
            client_secret: "whoop-test-secret".to_owned(),
            access_token: Some("whoop-access-do-not-log".to_owned()),
            refresh_token: None,
            expires_at: Some(Utc::now() + Duration::hours(1)),
            scopes: Vec::new(),
        })
        .await
        .unwrap();

    let activity = provider
        .get_activity("5b8e1c2a-0000-4000-8000-000000000539")
        .await
        .unwrap();

    assert_eq!(
        activity.training_stress_score(),
        None,
        "WHOOP's workout strain is never mapped to a training stress score"
    );
    assert_eq!(activity.sport_type(), &SportType::Run);
    assert_eq!(activity.duration_seconds(), 3_600);
    assert_eq!(activity.average_heart_rate(), Some(142));
    assert_eq!(activity.max_heart_rate(), Some(171));
    assert_eq!(activity.distance_meters(), Some(10_000.0));
    assert_eq!(activity.elevation_gain(), Some(85.0));
    // 2 100 kJ × 0.239 kcal/kJ, truncated.
    assert_eq!(activity.calories(), Some(501));
}

#[tokio::test]
async fn the_migration_clears_whoop_scores_already_stored_and_nothing_else() {
    let db = create_test_db().await;
    let repos = Arc::new(db.repositories());
    let (user_id, tenant) = seed_user(&db).await;
    let whoop_ds = data_source(&repos, user_id, tenant, "whoop").await;
    let garmin_ds = data_source(&repos, user_id, tenant, "garmin").await;

    // Rows as the pre-change sync wrote them: straight to the repositories,
    // WHOOP's scores included.
    let recent = whoop_night(user_id, &whoop_ds, night_start());
    let mut legacy = whoop_night(user_id, &whoop_ds, night_start() - Duration::days(1));
    legacy.awake_seconds = None;
    for night in [&recent, &legacy, &garmin_night(user_id, &garmin_ds)] {
        repos
            .sleep
            .upsert_sleep_session(&tenant, night)
            .await
            .unwrap();
    }
    for day in [
        whoop_day(user_id, &whoop_ds),
        garmin_day(user_id, &garmin_ds),
    ] {
        repos
            .recovery
            .upsert_recovery_metrics(&tenant, &day)
            .await
            .unwrap();
    }
    let run = |provider: &str, tss: f32| {
        ActivityBuilder::new(
            format!("{provider}-run-{user_id}"),
            format!("{provider} run"),
            SportType::Run,
            night_start() + Duration::hours(13),
            3_600,
            provider.to_owned(),
        )
        .distance_meters(10_000.0)
        .training_stress_score(tss)
        .build()
    };
    repos
        .activity_cache
        .upsert_activities(user_id, &tenant, "whoop", &[run("whoop", 12.3)])
        .await
        .unwrap();
    repos
        .activity_cache
        .upsert_activities(user_id, &tenant, "strava", &[run("strava", 80.0)])
        .await
        .unwrap();

    run_migration(&db).await;
    // Rerunning it changes nothing: every statement is a plain rewrite.
    run_migration(&db).await;

    let read = nights(&repos, user_id, tenant).await;
    assert_eq!(read.len(), 3, "{read:?}");
    let whoop_recent = read.iter().find(|n| n.id == recent.id).unwrap();
    assert_eq!(whoop_recent.sleep_score, None);
    assert_eq!(whoop_recent.sleep_efficiency, Some(DRAVR_EFFICIENCY));
    assert_eq!(whoop_recent.total_sleep_seconds, Some(25_200));
    assert_eq!(whoop_recent.avg_hrv, Some(71.0));
    let whoop_legacy = read.iter().find(|n| n.id == legacy.id).unwrap();
    assert_eq!(whoop_legacy.sleep_score, None);
    assert_eq!(
        whoop_legacy.sleep_efficiency, None,
        "without an awake time there is no Dravr efficiency, and WHOOP's is gone"
    );
    let garmin = from(&read, "garmin", |n| n.source_name.as_str());
    assert_eq!(garmin.sleep_score, Some(81));
    assert_eq!(garmin.sleep_efficiency, Some(90.0));

    let read = days(&repos, user_id, tenant).await;
    let whoop = from(&read, "whoop", |d| d.source_name.as_str());
    assert_eq!(whoop.recovery_score, None);
    assert_eq!(whoop.daily_strain, None);
    assert_eq!(whoop.hrv_rmssd, Some(61.5));
    assert_eq!(whoop.resting_heart_rate, Some(52));
    let garmin = from(&read, "garmin", |d| d.source_name.as_str());
    assert_eq!(garmin.recovery_score, Some(40));
    assert_eq!(garmin.body_battery, Some(60));

    let window = (night_start(), night_start() + Duration::days(1));
    let whoop_runs = repos
        .activity_cache
        .get_cached_activities(user_id, &tenant, Some("whoop"), window.0, window.1, 10)
        .await
        .unwrap();
    assert_eq!(whoop_runs.len(), 1);
    assert_eq!(whoop_runs[0].training_stress_score(), None);
    assert_eq!(whoop_runs[0].distance_meters(), Some(10_000.0));
    let strava_runs = repos
        .activity_cache
        .get_cached_activities(user_id, &tenant, Some("strava"), window.0, window.1, 10)
        .await
        .unwrap();
    assert_eq!(strava_runs.len(), 1);
    assert_eq!(strava_runs[0].training_stress_score(), Some(80.0));
}
