// ABOUTME: carnet#513 — health sync reaches scrape-connected Garmin and COROS athletes through their mirror sessions
// ABOUTME: Roster, credentials, tenant, stored rows, dead sessions, the scrape cadence and the sciotte-service reader

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The health sync knows a provider by its own name (`garmin`, `coros`),
//! while a scrape-connected athlete's session lives on the mirror backend's
//! token row (`sciotte_garmin`, `sciotte_coros`). Before this bridge the sync
//! looked the athlete up under the provider name, found nobody, and wrote
//! nothing — silently. These tests seed the rows exactly as the sciotte login
//! writes them and assert the values read back from storage.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
#![cfg(feature = "health-sync")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

#[path = "helpers/db_fixtures.rs"]
mod db_fixtures;

use std::env;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use db_fixtures::seed_user;
use pierre_core::errors::AppError;
use pierre_core::models::{
    OAuthNotification, RefreshConfig, StoredRecoveryMetrics, SyncStatus, TenantId, UserOAuthToken,
};
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db_with_key;
use pierre_database::RepositoryRegistry;
use pierre_enforme::models::connection::ConnectedUser;
use pierre_enforme::providers::sciotte_reader::{
    AuthSession, DailySummary, DailySummaryReader, ScraperError, ScraperResult,
};
use pierre_enforme::traits::connection_store::UserConnectionStore;
use pierre_enforme::traits::credential_store::CredentialStore;
use pierre_enforme::traits::cursor_store::SyncCursorStore;
use pierre_enforme::traits::recovery_store::RecoveryStore;
use pierre_providers::sciotte_remote::{ENV_AUDIENCE, ENV_REMOTE_URL};
use pierre_services::health_sync::PierreSyncStorage;
use pierre_services::provider_refresh::{scrape_sync_not_due, RefreshService, SyncNotifier};
use pierre_services::sciotte_health_reader::SciotteServiceReader;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use uuid::Uuid;

/// The session JSON the sciotte login stores as the token row's access token.
fn session() -> AuthSession {
    serde_json::from_str(
        r#"{"session_id":"sess-513","cookies":[{"name":"CPL-coros-token","value":"t",
            "domain":".coros.com","path":"/","secure":true,"http_only":false}],
            "created_at":"2026-09-20T00:00:00Z"}"#,
    )
    .unwrap()
}

async fn create_test_db() -> Database {
    create_test_db_with_key(b"scrape-health-sync-key-32-bytes!".to_vec())
        .await
        .unwrap()
}

/// A token row exactly as `store_sciotte_session` writes it.
async fn connect_scrape(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: TenantId,
    backend: &str,
) {
    let now = Utc::now();
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id: tenant.to_string(),
            provider: backend.to_owned(),
            access_token: serde_json::to_string(&session()).unwrap(),
            refresh_token: None,
            token_type: "session".to_owned(),
            expires_at: None,
            scope: None,
            provider_user_id: None,
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
}

/// An OAuth token row for an API provider.
async fn connect_oauth(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: TenantId,
    provider: &str,
) {
    let now = Utc::now();
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
            expires_at: Some(now + Duration::hours(1)),
            scope: Some("read".to_owned()),
            provider_user_id: None,
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
}

fn yesterday() -> NaiveDate {
    Utc::now().date_naive() - Duration::days(1)
}

fn window() -> (DateTime<Utc>, DateTime<Utc>) {
    (
        Utc::now() - Duration::days(30),
        Utc::now() + Duration::days(1),
    )
}

/// Test double for the scraper: the summary sciotte's `provider` config
/// returns for a day, as the service would serialize it. Records every
/// (provider, day) it is asked for. The real reader is exercised against a
/// stub service in `the_service_reader_*` below.
type Answer = Box<dyn Fn(&str, NaiveDate) -> ScraperResult<DailySummary> + Send + Sync>;

struct FakeScraper {
    answer: Answer,
    asked: Mutex<Vec<(String, NaiveDate)>>,
}

impl FakeScraper {
    fn reader(
        answer: impl Fn(&str, NaiveDate) -> ScraperResult<DailySummary> + Send + Sync + 'static,
    ) -> (Arc<Self>, Arc<dyn DailySummaryReader>) {
        let fake = Arc::new(Self {
            answer: Box::new(answer),
            asked: Mutex::new(Vec::new()),
        });
        let reader: Arc<dyn DailySummaryReader> = Arc::clone(&fake) as Arc<dyn DailySummaryReader>;
        (fake, reader)
    }
}

#[async_trait]
impl DailySummaryReader for FakeScraper {
    async fn daily_summary(
        &self,
        provider: &str,
        _session: &AuthSession,
        date: NaiveDate,
    ) -> ScraperResult<DailySummary> {
        self.asked.lock().unwrap().push((provider.to_owned(), date));
        (self.answer)(provider, date)
    }
}

fn summary(date: NaiveDate, provider: &str, fields: &str) -> DailySummary {
    let sep = if fields.is_empty() { "" } else { "," };
    serde_json::from_str(&format!(
        r#"{{"date":"{date}","provider":"{provider}"{sep}{fields}}}"#
    ))
    .unwrap()
}

/// COROS measured yesterday only; every other day of the window is empty,
/// as the Training Hub answers a day it holds nothing for.
fn coros_yesterday(provider: &str, date: NaiveDate) -> DailySummary {
    if date == yesterday() {
        summary(
            date,
            provider,
            r#""resting_heart_rate":51,"hrv_value":41,"vo2_max":49.0,"training_load":512,
               "fitness_score":88,"fatigue_score":84"#,
        )
    } else {
        summary(date, provider, "")
    }
}

/// Garmin measured yesterday's night and morning.
fn garmin_yesterday(provider: &str, date: NaiveDate) -> DailySummary {
    if date == yesterday() {
        summary(
            date,
            provider,
            r#""sleep_duration_seconds":27000,"sleep_deep_seconds":5400,"sleep_score":82,
               "hrv_value":62,"resting_heart_rate":47,"body_battery":71,"weight_kg":70.5,
               "vo2_max":53.0"#,
        )
    } else {
        summary(date, provider, "")
    }
}

fn dead_session(_: &str, _: NaiveDate) -> ScraperResult<DailySummary> {
    Err(ScraperError::SessionExpired {
        reason: "access token is invalid".to_owned(),
    })
}

#[tokio::test]
async fn a_coros_athlete_is_on_the_roster_under_coros_with_its_session() {
    let db = create_test_db().await;
    let (user_id, tenant) = seed_user(&db).await;
    let repos = Arc::new(db.repositories());
    connect_scrape(&repos, user_id, tenant, "sciotte_coros").await;
    let storage = PierreSyncStorage::new(&repos);

    let roster = storage.list_connected_users("coros").await.unwrap();
    assert_eq!(roster.len(), 1);
    assert_eq!(roster[0].user_id, user_id.to_string());
    assert_eq!(roster[0].provider, "coros");
    // Nobody is connected to Garmin.
    assert!(storage
        .list_connected_users("garmin")
        .await
        .unwrap()
        .is_empty());

    let creds = storage
        .get_credentials(&user_id.to_string(), "coros")
        .await
        .unwrap()
        .expect("the sciotte_coros session serves coros");
    let restored: AuthSession = serde_json::from_str(&creds.access_token).unwrap();
    assert_eq!(restored.session_id, "sess-513");
}

#[tokio::test]
async fn a_coros_sync_stores_resting_hr_sleep_hrv_and_vo2max_as_coros() {
    let db = create_test_db().await;
    let (user_id, tenant) = seed_user(&db).await;
    let repos = Arc::new(db.repositories());
    connect_scrape(&repos, user_id, tenant, "sciotte_coros").await;
    let storage = Arc::new(PierreSyncStorage::new(&repos));
    let (fake, reader) = FakeScraper::reader(|p, d| Ok(coros_yesterday(p, d)));
    let orchestrator = storage.build_orchestrator_with_reader(&reader);

    let result = orchestrator
        .sync_user(&user_id.to_string(), "coros")
        .await
        .unwrap();

    assert_eq!(result.status, SyncStatus::Completed);
    // One recovery row and one health row: the empty days write nothing.
    assert_eq!(result.records_created, 2);

    let (start, end) = window();
    let recovery = repos
        .recovery
        .get_recovery_metrics(user_id, &tenant, start, end)
        .await
        .unwrap();
    assert_eq!(recovery.len(), 1);
    let day = &recovery[0];
    assert_eq!(day.source_name, "coros");
    assert_eq!(day.date, yesterday());
    assert_eq!(day.resting_heart_rate, Some(51));
    assert_eq!(day.hrv_rmssd, Some(41.0));
    assert_eq!(day.hrv_ms, Some(41.0));
    // COROS's 7-day load, CTI and ATI are its load model, not readings.
    assert_eq!(day.recovery_score, None);
    assert_eq!(day.stress_score, None);
    assert_eq!(day.daily_strain, None);

    let health = repos
        .health_snapshots
        .get_health_snapshots(user_id, &tenant, start, end)
        .await
        .unwrap();
    assert_eq!(health.len(), 1);
    assert_eq!(health[0].source_name, "coros");
    assert_eq!(health[0].vo2_max, Some(49.0));
    assert_eq!(health[0].weight_kg, None);

    // Every page was asked of sciotte's `coros` config, over two weeks.
    let asked = fake.asked.lock().unwrap().clone();
    assert!(asked.iter().all(|(p, _)| p == "coros"));
    assert!(asked.iter().any(|(_, d)| *d == yesterday()));

    // The cursor is stored under the sync's name and stops at today.
    let cursor = storage
        .get_cursor(&user_id.to_string(), "coros", "recovery")
        .await
        .unwrap()
        .expect("a recovery cursor is stored");
    assert_eq!(cursor.value, Utc::now().date_naive().to_string());
}

#[tokio::test]
async fn a_garmin_sync_stores_the_night_the_morning_and_the_body_as_garmin() {
    let db = create_test_db().await;
    let (user_id, tenant) = seed_user(&db).await;
    let repos = Arc::new(db.repositories());
    connect_scrape(&repos, user_id, tenant, "sciotte_garmin").await;
    let storage = Arc::new(PierreSyncStorage::new(&repos));
    let (_fake, reader) = FakeScraper::reader(|p, d| Ok(garmin_yesterday(p, d)));
    let orchestrator = storage.build_orchestrator_with_reader(&reader);

    let result = orchestrator
        .sync_user(&user_id.to_string(), "garmin")
        .await
        .unwrap();
    assert_eq!(result.status, SyncStatus::Completed);
    assert_eq!(result.records_created, 3);

    let (start, end) = window();
    let nights = repos
        .sleep
        .get_sleep_sessions(user_id, &tenant, start, end)
        .await
        .unwrap();
    assert_eq!(nights.len(), 1);
    assert_eq!(nights[0].source_name, "garmin");
    assert_eq!(nights[0].total_sleep_seconds, Some(27_000));
    assert_eq!(nights[0].deep_sleep_seconds, Some(5_400));

    let mornings = repos
        .recovery
        .get_recovery_metrics(user_id, &tenant, start, end)
        .await
        .unwrap();
    assert_eq!(mornings.len(), 1);
    assert_eq!(mornings[0].hrv_rmssd, Some(62.0));
    assert_eq!(mornings[0].resting_heart_rate, Some(47));

    let body = repos
        .health_snapshots
        .get_health_snapshots(user_id, &tenant, start, end)
        .await
        .unwrap();
    assert_eq!(body.len(), 1);
    assert_eq!(body[0].vo2_max, Some(53.0));
    assert!((body[0].weight_kg.unwrap() - 70.5).abs() < 0.01);
}

#[tokio::test]
async fn a_dead_coros_session_writes_nothing_and_keeps_no_cursor() {
    let db = create_test_db().await;
    let (user_id, tenant) = seed_user(&db).await;
    let repos = Arc::new(db.repositories());
    connect_scrape(&repos, user_id, tenant, "sciotte_coros").await;
    let storage = Arc::new(PierreSyncStorage::new(&repos));
    let (fake, reader) = FakeScraper::reader(dead_session);
    let orchestrator = storage.build_orchestrator_with_reader(&reader);

    let result = orchestrator
        .sync_user(&user_id.to_string(), "coros")
        .await
        .unwrap();

    assert_eq!(result.status, SyncStatus::Failed);
    assert_eq!(result.records_created, 0);
    assert_eq!(result.records_errored, 2);
    let (start, end) = window();
    assert!(repos
        .recovery
        .get_recovery_metrics(user_id, &tenant, start, end)
        .await
        .unwrap()
        .is_empty());
    assert!(storage
        .get_cursor(&user_id.to_string(), "coros", "recovery")
        .await
        .unwrap()
        .is_none());
    // Recovery stops at its first refused page and retries it once after the
    // one credential refresh a sync allows; health stops at its first page:
    // three pages, not one per day of the window.
    assert_eq!(fake.asked.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn no_row_is_written_under_another_providers_tenant() {
    let db = create_test_db().await;
    let (user_id, tenant) = seed_user(&db).await;
    let repos = Arc::new(db.repositories());
    // The athlete holds a WHOOP grant and no COROS connection.
    connect_oauth(&repos, user_id, tenant, "whoop").await;
    let storage = Arc::new(PierreSyncStorage::new(&repos));

    let stray = StoredRecoveryMetrics {
        id: format!("coros-recovery-{user_id}-{}", yesterday()),
        user_id: user_id.to_string(),
        data_source_id: "coros-default".to_owned(),
        date: yesterday(),
        recovery_score: None,
        readiness_score: None,
        hrv_ms: Some(41.0),
        hrv_rmssd: Some(41.0),
        resting_heart_rate: Some(51),
        stress_score: None,
        body_battery: None,
        spo2: None,
        respiratory_rate: None,
        skin_temp_deviation: None,
        daily_strain: None,
        athlete_note: None,
        source_name: "coros".to_owned(),
        recorded_at: Utc::now(),
    };
    let err = storage.store_recovery_metrics(&[stray]).await.unwrap_err();
    assert!(
        err.to_string().contains("No token serves provider 'coros'"),
        "{err}"
    );

    let (start, end) = window();
    assert!(repos
        .recovery
        .get_recovery_metrics(user_id, &tenant, start, end)
        .await
        .unwrap()
        .is_empty());
    // And the sync has no credentials for it.
    assert!(storage
        .get_credentials(&user_id.to_string(), "coros")
        .await
        .is_err());
}

#[tokio::test]
async fn a_scrape_sync_waits_six_hours_and_an_api_sync_never_waits() {
    let db = create_test_db().await;
    let (user_id, tenant) = seed_user(&db).await;
    let repos = Arc::new(db.repositories());
    connect_scrape(&repos, user_id, tenant, "sciotte_coros").await;
    connect_oauth(&repos, user_id, tenant, "whoop").await;
    let auth = repos.auth_repos();
    let coros = ConnectedUser::new(user_id.to_string(), "coros");
    let whoop = ConnectedUser::new(user_id.to_string(), "whoop");

    // Never synced: due.
    assert!(!scrape_sync_not_due(&auth, &coros, "coros").await);

    repos
        .oauth_tokens
        .update_provider_last_sync(user_id, tenant, "sciotte_coros", Utc::now())
        .await
        .unwrap();
    assert!(scrape_sync_not_due(&auth, &coros, "coros").await);

    repos
        .oauth_tokens
        .update_provider_last_sync(
            user_id,
            tenant,
            "sciotte_coros",
            Utc::now() - Duration::hours(7),
        )
        .await
        .unwrap();
    assert!(!scrape_sync_not_due(&auth, &coros, "coros").await);

    // An API provider is polled every cycle, whatever its last sync.
    repos
        .oauth_tokens
        .update_provider_last_sync(user_id, tenant, "whoop", Utc::now())
        .await
        .unwrap();
    assert!(!scrape_sync_not_due(&auth, &whoop, "whoop").await);
}

/// The refresh service needs a notifier; these tests assert on what was
/// scraped, not on SSE delivery.
struct NoopNotifier;

#[async_trait]
impl SyncNotifier for NoopNotifier {
    async fn send_notification(
        &self,
        _user_id: Uuid,
        _notification: &OAuthNotification,
    ) -> Result<(), AppError> {
        Ok(())
    }
}

#[tokio::test]
async fn a_chat_turn_never_scrapes_for_a_garmin_session_or_a_leftover_garmin_row() {
    let db = create_test_db().await;
    let (user_id, tenant) = seed_user(&db).await;
    let repos = Arc::new(db.repositories());
    connect_scrape(&repos, user_id, tenant, "sciotte_garmin").await;
    // A leftover row for Garmin's partner-gated OAuth API, which the Garmin
    // sync does not read and whose last_sync nothing stamps.
    connect_oauth(&repos, user_id, tenant, "garmin").await;
    let storage = Arc::new(PierreSyncStorage::new(&repos));
    let (fake, reader) = FakeScraper::reader(|p, d| Ok(garmin_yesterday(p, d)));
    let service = RefreshService::new(
        &repos.auth_repos(),
        repos.activity_cache.clone(),
        Some(storage.build_orchestrator_with_reader(&reader)),
        Arc::new(NoopNotifier) as Arc<dyn SyncNotifier>,
    );

    let status = service
        .check_and_refresh(user_id, tenant, &RefreshConfig::default())
        .await;

    // Both rows are judged by the activity cache, which is cold: stale,
    // reported for get_activities to re-fetch, and nothing scraped in the turn.
    assert!(
        status.refreshing.iter().any(|p| p == "garmin"),
        "{status:?}"
    );
    assert!(
        status.refreshing.iter().any(|p| p == "sciotte_garmin"),
        "{status:?}"
    );
    assert!(fake.asked.lock().unwrap().is_empty());
}

/// Every request the stub scraper service received, in order.
type Seen = Arc<Mutex<Vec<String>>>;

/// A scraper service that imports any session and answers the daily summary
/// with `status` and `body`.
fn spawn_scraper_stub(listener: TcpListener, seen: Seen, status: &'static str, body: &'static str) {
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = vec![0_u8; 16384];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            seen.lock().unwrap().push(request.clone());
            let (status, body) = if request.contains("/auth/import-session") {
                ("200 OK", r#"{"session_id":"sess-513"}"#)
            } else {
                (status, body)
            };
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
}

async fn stub_service(status: &'static str, body: &'static str) -> Seen {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    spawn_scraper_stub(listener, Arc::clone(&seen), status, body);
    env::set_var(ENV_REMOTE_URL, format!("http://{addr}"));
    env::remove_var(ENV_AUDIENCE);
    seen
}

/// One test drives every service answer in turn: the service URL is process
/// state, so the cases must not run concurrently.
#[tokio::test]
async fn the_service_reader_imports_the_session_and_maps_every_answer() {
    let day = NaiveDate::from_ymd_opt(2026, 9, 22).unwrap();

    // A measured day.
    let seen = stub_service(
        "200 OK",
        r#"{"date":"2026-09-22","provider":"coros","resting_heart_rate":51,"hrv_value":41,"vo2_max":49.0}"#,
    )
    .await;
    let summary = SciotteServiceReader
        .daily_summary("coros", &session(), day)
        .await
        .unwrap();
    assert_eq!(summary.date, day);
    assert_eq!(summary.resting_heart_rate, Some(51));
    assert_eq!(summary.hrv_value, Some(41));
    assert_eq!(summary.vo2_max, Some(49.0));
    let requests = seen.lock().unwrap().clone();
    assert!(requests[0].contains("/auth/import-session"));
    assert!(
        requests[0].contains(r#""provider":"coros""#),
        "{}",
        requests[0]
    );
    assert!(
        requests[1].contains("GET /api/daily-summary?date=2026-09-22"),
        "{}",
        requests[1]
    );
    assert!(
        requests[1].contains("sess-513"),
        "the scrape names the imported session"
    );

    // A dead session is expired, not an empty day.
    stub_service(
        "401 Unauthorized",
        r#"{"error":"session_expired","message":"access token is invalid"}"#,
    )
    .await;
    let dead = SciotteServiceReader
        .daily_summary("coros", &session(), day)
        .await
        .unwrap_err();
    assert!(
        matches!(dead, ScraperError::SessionExpired { .. }),
        "{dead:?}"
    );

    // A load-shed carries the service's own wait.
    stub_service(
        "503 Service Unavailable",
        r#"{"error":"scraper_busy","reason":"queue full","retry_after_secs":9}"#,
    )
    .await;
    match SciotteServiceReader
        .daily_summary("coros", &session(), day)
        .await
        .unwrap_err()
    {
        ScraperError::Busy {
            retry_after_secs, ..
        } => assert_eq!(retry_after_secs, 9),
        other => panic!("expected Busy, got {other:?}"),
    }
}
