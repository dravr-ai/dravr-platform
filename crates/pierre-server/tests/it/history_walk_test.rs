// ABOUTME: A usr/pwd Garmin sign-in beside a Garmin grant walks the whole history, 36 months a step, and keeps the session
// ABOUTME: Pins the walk under the session key, its steps and progress, the silent incomplete end, the callback, the resume

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! History walks of a session beside a Garmin grant (carnet#1482).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// The Garmin grant is the athlete's one ongoing source. A usr/pwd sign-in
// beside it is inferred, on the server, to be walk-only: a history walk pages
// the session itself back, 36 months a step, stores what it reads under the
// session's key (served through the grant's window), and ends at the first
// empty step, keeping its row as walked. The session stays (B1). A session
// that dies mid-walk ends it incomplete, which the card reads and the athlete
// is never told as a reconnect. A Garmin OAuth connect walks a live session
// already standing beside the new grant and retires a dead one. A
// usr/pwd-only athlete is untouched.
//
// The scraper is a loopback stand-in (a test double, per the repo's mock
// rule) that answers by session and records every call it serves; Garmin's
// OAuth is the dev fixture's stand-in, as in the Garmin API e2e suite.
#![cfg(all(feature = "provider-garmin", feature = "health-sync"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers::axum_test::AxumTestRequest;
use crate::helpers::sciotte_mock::seed_scrape_session;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use chrono::{Duration as ChronoDuration, NaiveDate, Utc};
use pierre_config::environment::{ProviderEndpointOverride, ServerConfig};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{
    Activity, ActivityBuilder, ConnectionStatus, ConnectionType, DataSource, DeviceType, SportType,
    StoredRecoveryMetrics, TenantId, User, UserOAuthToken, PERMISSION_WITHDRAWN,
};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::{
    ActivityBackfillJobRow, BackfillJobClaim, BackfillJobKind, BackfillJobState, JobLease,
    JobProgress, WellnessWriteMode,
};
use pierre_dev_fixture::garmin::{self, GarminFixture, GarminFixtureConfig};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_notifications::{NotificationService, TenantId as CommereTenantId};
use pierre_providers::core::ActivityQueryParams;
use pierre_routes_auth::AuthRoutes;
use pierre_services::oauth_flow::{AuthUrlOptions, OAuthService};
use pierre_services::provider_revocation::retire_history_session;
use pierre_tool_runtime::activity_backfill::history_walk::{
    SCIOTTE_WALK_STEP_MONTHS, WHOLE_ACCOUNT_AFTER,
};
use pierre_tool_runtime::activity_backfill_resume::resume_backfill_jobs;
use pierre_tool_runtime::activity_fetch::fetch_provider_activities;
use pierre_tool_runtime::protocol::AuthService;
use pierre_tool_runtime::runtime::ToolRuntime;
use reqwest::header::LOCATION;
use reqwest::redirect::Policy;
use secrecy::SecretString;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::time::sleep;
use tokio_util::task::TaskTracker;
use url::Url;
use uuid::Uuid;

/// The session the stand-in signs every credential login in to.
pub(crate) const SIGNED_IN_SESSION: &str = "history-session";
/// A session seeded live beside a grant the callback is about to land.
pub(crate) const LIVE_SESSION: &str = "live-session";
/// A session seeded flagged beside a grant the callback is about to land.
pub(crate) const DEAD_SESSION: &str = "dead-session";

/// The three workouts the stand-in's account holds, oldest first: one in
/// each of the walk's first three 36-month steps, so the fourth step finds
/// nothing and ends the walk.
const OLD_RUN: &str = "7000000001";
const MID_RIDE: &str = "7000000002";
const RECENT_RIDE: &str = "7000000003";

/// How long a detached walk gets to end before the test gives up.
pub(crate) const IMPORT_DEADLINE: Duration = Duration::from_secs(20);

/// Every call the stand-in served, as `"<route> <session or detail>"`.
pub(crate) type Calls = Arc<Mutex<Vec<String>>>;

/// The stand-in scraper's state: what it served, whether a read of the
/// account finds the session expired, and whether the next one fails.
#[derive(Clone)]
struct Scraper {
    calls: Calls,
    switch: ScraperSwitch,
}

/// What a test flips on the stand-in while it runs.
#[derive(Clone, Default)]
pub(crate) struct ScraperSwitch {
    /// Every read of the account answers that the session expired.
    pub(crate) expired: Arc<AtomicBool>,
    /// The next activity read answers a server error, once.
    pub(crate) fail_next: Arc<AtomicBool>,
    /// Every daily-summary read answers a server error.
    pub(crate) fail_summaries: Arc<AtomicBool>,
    /// The daily-summary read of this one day answers a server error.
    pub(crate) fail_summary_on: Arc<Mutex<Option<NaiveDate>>>,
}

fn session_header(headers: &HeaderMap) -> String {
    headers
        .get("x-session-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

fn session_json(session_id: &str) -> Value {
    json!({
        "session_id": session_id,
        "cookies": [],
        "created_at": "2026-11-04T12:00:00Z",
        "expires_at": null
    })
}

fn workout(id: &str, sport: &str, start: &str) -> Value {
    json!({
        "id": id,
        "name": format!("Garmin {sport}"),
        "sport_type": sport,
        "start_date": start,
        "duration_seconds": 3600,
        "provider": "garmin",
        "distance_meters": 12000.0
    })
}

/// A stand-in for the scraper service over a Garmin account holding a run
/// eight years old, a ride four years old and a ride three days old, served
/// within each read's `after`/`before` bounds. `expired` makes every read of
/// the account answer that the session expired, as the ~2 h Garmin web
/// session does mid-copy.
async fn spawn_scraper(expired: bool) -> (String, Calls) {
    let switch = ScraperSwitch::default();
    switch.expired.store(expired, Ordering::SeqCst);
    let (url, calls) = spawn_switchable_scraper(switch).await;
    (url, calls)
}

/// [`spawn_scraper`], answering as `switch` says at each read. Every day's
/// summary holds a resting heart rate.
pub(crate) async fn spawn_switchable_scraper(switch: ScraperSwitch) -> (String, Calls) {
    let calls: Calls = Arc::new(Mutex::new(Vec::new()));
    let state = Scraper {
        calls: Arc::clone(&calls),
        switch,
    };
    let now = Utc::now();
    let account = Arc::new(vec![
        (RECENT_RIDE, "ride", now - ChronoDuration::days(3)),
        (MID_RIDE, "ride", now - ChronoDuration::days(4 * 365)),
        (OLD_RUN, "run", now - ChronoDuration::days(8 * 365)),
    ]);
    let app = Router::new()
        .route(
            "/auth/login-with-credentials",
            post(
                |State(scraper): State<Scraper>, Json(body): Json<Value>| async move {
                    scraper
                        .calls
                        .lock()
                        .unwrap()
                        .push(format!("login {}", body["provider"]));
                    Json(json!({
                        "status": "authenticated",
                        "session_id": SIGNED_IN_SESSION,
                        "provider": body["provider"],
                    }))
                },
            ),
        )
        .route(
            "/auth/sessions/{session_id}/export",
            get(|Path(session_id): Path<String>| async move {
                Json(json!({ "session": session_json(&session_id) }))
            }),
        )
        .route(
            "/auth/sessions/{session_id}",
            delete(
                |State(scraper): State<Scraper>, Path(session_id): Path<String>| async move {
                    scraper
                        .calls
                        .lock()
                        .unwrap()
                        .push(format!("drop {session_id}"));
                    StatusCode::NO_CONTENT
                },
            ),
        )
        .route(
            "/auth/import-session",
            post(|Json(body): Json<Value>| async move {
                Json(json!({ "session_id": body["session"]["session_id"] }))
            }),
        )
        .route(
            "/api/athlete",
            get(|| async { Json(json!({ "id": "garmin-athlete", "display_name": "Gar Min" })) }),
        )
        .route(
            "/api/activities",
            get(
                move |State(scraper): State<Scraper>,
                      headers: HeaderMap,
                      RawQuery(query): RawQuery| {
                    let account = Arc::clone(&account);
                    async move {
                        let session = session_header(&headers);
                        let query = query.unwrap_or_default();
                        scraper
                            .calls
                            .lock()
                            .unwrap()
                            .push(format!("activities {session} {query}"));
                        if scraper.switch.fail_next.swap(false, Ordering::SeqCst) {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(json!({ "error": "internal", "message": "scraper hiccup" })),
                            );
                        }
                        if scraper.switch.expired.load(Ordering::SeqCst) {
                            return (
                                StatusCode::UNAUTHORIZED,
                                Json(json!({
                                    "error": "session_expired",
                                    "message": "the Garmin web session expired"
                                })),
                            );
                        }
                        let after = query_bound(&query, "after");
                        let before = query_bound(&query, "before");
                        let activities: Vec<Value> = account
                            .iter()
                            .filter(|(_, _, start)| {
                                after.is_none_or(|after| start.timestamp() >= after)
                                    && before.is_none_or(|before| start.timestamp() < before)
                            })
                            .map(|(id, sport, start)| workout(id, sport, &start.to_rfc3339()))
                            .collect();
                        (
                            StatusCode::OK,
                            Json(json!({
                                "count": activities.len(),
                                "activities": activities,
                                "head_complete": true
                            })),
                        )
                    }
                },
            ),
        )
        .route(
            "/api/daily-summary",
            get(
                |State(scraper): State<Scraper>, headers: HeaderMap, RawQuery(query): RawQuery| async move {
                    let session = session_header(&headers);
                    let query = query.unwrap_or_default();
                    let date = query
                        .split('&')
                        .find_map(|pair| pair.strip_prefix("date="))
                        .unwrap_or_default()
                        .to_owned();
                    scraper
                        .calls
                        .lock()
                        .unwrap()
                        .push(format!("summary {session} {date}"));
                    if scraper.switch.expired.load(Ordering::SeqCst) {
                        return (
                            StatusCode::UNAUTHORIZED,
                            Json(json!({
                                "error": "session_expired",
                                "message": "the Garmin web session expired"
                            })),
                        );
                    }
                    let failing_day = *scraper.switch.fail_summary_on.lock().unwrap();
                    if scraper.switch.fail_summaries.load(Ordering::SeqCst)
                        || failing_day.is_some_and(|day| day.to_string() == date)
                    {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({ "error": "internal", "message": "scraper hiccup" })),
                        );
                    }
                    (
                        StatusCode::OK,
                        Json(json!({ "date": date, "provider": "garmin", "resting_heart_rate": 48 })),
                    )
                },
            ),
        )
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), calls)
}

/// The epoch-seconds bound `name` an activities read's query names.
fn query_bound(query: &str, name: &str) -> Option<i64> {
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix(&format!("{name}=")))
        .and_then(|value| value.parse().ok())
}

pub(crate) fn calls_matching(calls: &Calls, prefix: &str) -> Vec<String> {
    calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| call.starts_with(prefix))
        .cloned()
        .collect()
}

/// A server over `config` whose notification service writes to its own
/// database, so the notices a walk sends (none) can be counted.
pub(crate) async fn server(config: ServerConfig) -> Arc<ServerContext> {
    common::init_server_config();
    common::init_test_http_clients();
    let base = common::create_test_server_resources_with_config(config)
        .await
        .unwrap();
    let mut with_notices: ServerContext = (*base).clone();
    let service = Arc::new(match with_notices.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            NotificationService::from_sqlite(sqlite.pool().clone(), TaskTracker::new())
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            NotificationService::from_postgres(pg.pool().clone(), TaskTracker::new())
        }
    });
    with_notices.common.notification_service = Some(service);
    Arc::new(with_notices)
}

/// A new athlete of `resources`, the tenant they own, and a bearer token.
pub(crate) async fn athlete(resources: &Arc<ServerContext>) -> (Uuid, User, TenantId, String) {
    let (user_id, user, tenant) = common::create_test_user_with_plan(
        &resources.agent.database,
        &format!("garmin-history-{}@example.com", Uuid::new_v4()),
        "starter",
    )
    .await
    .unwrap();
    let token = common::generate_test_token(resources, &user).await;
    (user_id, user, tenant, token)
}

/// A live Garmin grant and its connection, as the OAuth callback leaves them.
pub(crate) async fn hold_grant(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId) {
    let now = Utc::now();
    let repos = &resources.common.repos;
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id: tenant.to_string(),
            provider: oauth_providers::GARMIN.to_owned(),
            access_token: SecretString::from("garmin-access-do-not-log".to_owned()),
            refresh_token: Some(SecretString::from("garmin-refresh-do-not-log".to_owned())),
            token_type: "Bearer".to_owned(),
            expires_at: Some(now + ChronoDuration::hours(12)),
            scope: None,
            provider_user_id: Some(format!("garmin-{user_id}")),
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    repos
        .provider_connections
        .register_connection(
            user_id,
            tenant,
            oauth_providers::GARMIN,
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
}

/// A usr/pwd Garmin session and its connection, as the hosted login writes
/// them, under `session_id`.
pub(crate) async fn hold_session(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    session_id: &str,
) {
    seed_scrape_session(
        resources,
        user_id,
        tenant,
        oauth_providers::SCIOTTE_GARMIN,
        session_id,
    )
    .await;
    resources
        .common
        .repos
        .provider_connections
        .register_connection(
            user_id,
            tenant,
            oauth_providers::SCIOTTE_GARMIN,
            &ConnectionType::Manual,
            None,
        )
        .await
        .unwrap();
}

/// Sign in to Garmin with a username and password, as the login modal does.
pub(crate) async fn sign_in(resources: &Arc<ServerContext>, token: &str) -> (u16, Value) {
    let resp = AxumTestRequest::post("/api/providers/sciotte/login")
        .header("authorization", &format!("Bearer {token}"))
        .json(&json!({
            "email": "athlete@example.com",
            "password": "not-a-real-password",
            "target": "garmin",
            "tos_consent": true,
        }))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    (
        resp.status(),
        serde_json::from_str(&resp.text()).unwrap_or(Value::Null),
    )
}

pub(crate) async fn session_token_held(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> bool {
    resources
        .common
        .repos
        .oauth_tokens
        .get_token(user_id, tenant, oauth_providers::SCIOTTE_GARMIN)
        .await
        .unwrap()
        .is_some()
}

/// What the ledger holds for the session's history walk.
pub(crate) async fn job_state(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> Option<BackfillJobState> {
    job_state_of(resources, user_id, tenant, BackfillJobKind::HistoryWalk).await
}

/// What the ledger holds for the session's `kind` job.
pub(crate) async fn job_state_of(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    kind: BackfillJobKind,
) -> Option<BackfillJobState> {
    resources
        .common
        .repos
        .activity_backfill_jobs
        .backfill_job_state(user_id, tenant, oauth_providers::SCIOTTE_GARMIN, kind)
        .await
        .unwrap()
}

/// Make every job row of the athlete old enough for the resume sweep, which
/// leaves a row alone for two minutes after it was asked so it never races
/// the ask's own spawn.
pub(crate) async fn age_job_rows(resources: &Arc<ServerContext>, user_id: Uuid) {
    let sql = "UPDATE activity_backfill_jobs SET created_at_ms = 0 WHERE user_id = $1";
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query(sql)
                .bind(user_id.to_string())
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            sqlx::query(sql)
                .bind(user_id.to_string())
                .execute(pg.pool())
                .await
                .unwrap();
        }
    }
}

/// Run the athlete's walk the way the resume sweep does, one step a pass,
/// until its row ends — walked, or incomplete — and return how it ended.
pub(crate) async fn walk_ended(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> BackfillJobState {
    let deadline = Instant::now() + IMPORT_DEADLINE;
    loop {
        let state = job_state(resources, user_id, tenant).await;
        if let Some(ended @ (BackfillJobState::Walked | BackfillJobState::Incomplete)) = state {
            return ended;
        }
        assert!(
            Instant::now() < deadline,
            "the walk did not end within {IMPORT_DEADLINE:?} (row: {state:?})"
        );
        age_job_rows(resources, user_id).await;
        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        resume_backfill_jobs(runtime).await.unwrap();
        sleep(Duration::from_millis(50)).await;
    }
}

/// The walk's progress on its row.
pub(crate) async fn walk_progress(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> JobProgress {
    resources
        .common
        .repos
        .activity_backfill_jobs
        .job_progress(
            user_id,
            tenant,
            oauth_providers::SCIOTTE_GARMIN,
            BackfillJobKind::HistoryWalk,
        )
        .await
        .unwrap()
        .expect("the walk's row is kept")
}

/// The athlete's cached activities under `key`.
pub(crate) async fn rows_under(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    key: &str,
) -> i64 {
    resources
        .common
        .repos
        .activity_cache
        .count_by_provider(user_id, &tenant)
        .await
        .unwrap()
        .into_iter()
        .find(|(provider, _)| provider == key)
        .map_or(0, |(_, count)| count)
}

/// The connections the athlete holds, by provider.
pub(crate) async fn connections(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> Vec<(String, ConnectionStatus)> {
    resources
        .common
        .repos
        .provider_connections
        .get_for_user(user_id, Some(tenant))
        .await
        .unwrap()
        .into_iter()
        .map(|c| (c.provider, c.status))
        .collect()
}

/// The type of every notification the athlete was sent.
pub(crate) async fn notification_types(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> Vec<String> {
    let service = resources
        .common
        .notification_service
        .as_ref()
        .expect("the test server wires notifications");
    let (rows, _, _) = service
        .list_notifications(
            user_id,
            CommereTenantId(tenant.as_uuid()),
            50,
            0,
            None,
            false,
        )
        .await
        .unwrap();
    rows.into_iter().map(|row| row.notification_type).collect()
}

/// How many reconnect notices the athlete was sent.
pub(crate) async fn reconnect_notices(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> usize {
    let service = resources
        .common
        .notification_service
        .as_ref()
        .expect("the test server wires notifications");
    let (rows, _, _) = service
        .list_notifications(
            user_id,
            CommereTenantId(tenant.as_uuid()),
            50,
            0,
            None,
            false,
        )
        .await
        .unwrap();
    rows.iter()
        .filter(|row| row.notification_type == "provider_needs_reauth")
        .count()
}

/// The Garmin card's `history`.
pub(crate) async fn garmin_history(resources: &Arc<ServerContext>, token: &str) -> Value {
    let resp = AxumTestRequest::get("/api/providers")
        .header("authorization", &format!("Bearer {token}"))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    let body: Value = serde_json::from_str(&resp.text()).unwrap();
    body["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|card| card["provider"] == oauth_providers::SCIOTTE_GARMIN)
        .expect("the Garmin card is served")["history"]
        .clone()
}

/// The archive source the Garmin card names: Garmin's export, when this
/// server offers it.
pub(crate) fn offered_garmin_archive(resources: &Arc<ServerContext>) -> Value {
    if resources
        .auth_routes_context()
        .offered_archive_sources
        .contains(&oauth_providers::GARMIN_ARCHIVE)
    {
        json!(oauth_providers::GARMIN_ARCHIVE)
    } else {
        Value::Null
    }
}

/// The activity ids a Garmin read serves the athlete, from 2010 on.
async fn served_ids(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> Vec<String> {
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let params = ActivityQueryParams {
        limit: Some(50),
        offset: None,
        before: None,
        after: Some(1_262_304_000),
    };
    let mut ids: Vec<String> = fetch_provider_activities(
        &runtime,
        oauth_providers::GARMIN,
        user_id,
        &tenant.to_string(),
        &params,
    )
    .await
    .expect("the grant's window answers")
    .iter()
    .map(|activity| Activity::id(activity).to_owned())
    .collect();
    ids.sort();
    ids
}

/// The dev athlete's path: a Garmin grant holder signs in with their
/// username and password. The server infers a walk from the grant: the
/// session itself is read 36 months a step, newest first — every step of it
/// the session's, never the grant's — its rows land under `sciotte_garmin`
/// and are served through the grant's window, and the first empty step ends
/// the walk, whose row is kept as walked with its progress. The session stays
/// for the next walk (B1), and nothing was told.
#[tokio::test]
async fn a_grant_holders_sign_in_walks_the_whole_history_and_keeps_the_session() {
    let (scraper, calls) = spawn_scraper(false).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;

    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        walk_ended(&resources, user_id, tenant).await,
        BackfillJobState::Walked,
        "bug: the walk reaches the end of the history"
    );

    assert_eq!(
        rows_under(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        3,
        "bug: the walk stores the whole account under the session's key"
    );
    let reads = calls_matching(&calls, &format!("activities {SIGNED_IN_SESSION}"));
    assert_eq!(
        reads.len(),
        4,
        "bug: three steps land a workout each and the fourth, empty, ends the walk: {reads:?}"
    );
    assert!(
        !reads[0].contains("before="),
        "bug: the first step reads the head of the list: {reads:?}"
    );
    assert!(
        reads[1..].iter().all(|read| read.contains("before=")),
        "bug: every later step reads below the last one: {reads:?}"
    );
    assert!(
        reads
            .iter()
            .all(|read| !read.contains(&format!("after={WHOLE_ACCOUNT_AFTER}"))),
        "bug: a step reads 36 months, never the whole account at once: {reads:?}"
    );
    let progress = walk_progress(&resources, user_id, tenant).await;
    assert_eq!(
        progress.months_done,
        4 * SCIOTTE_WALK_STEP_MONTHS,
        "bug: each step adds its 36 months to the walk's progress"
    );
    assert_eq!(
        progress.rows_landed, 3,
        "bug: the progress counts every row the steps landed"
    );
    assert!(
        progress.secs_per_unit.is_some(),
        "bug: a step measures the seconds a month took"
    );
    assert_eq!(
        progress.months_total, None,
        "bug: a session names no floor of its own, so the walk's length is unknown"
    );
    let coverage = resources
        .common
        .repos
        .activity_cache
        .get_backfill_coverage(user_id, &tenant, oauth_providers::SCIOTTE_GARMIN)
        .await
        .unwrap()
        .expect("the walk records its coverage");
    assert!(
        coverage.hit_feed_end,
        "bug: a walk that reached the end of the history says so"
    );
    assert!(
        coverage.oldest_reached_ts < (Utc::now() - ChronoDuration::days(8 * 365)).timestamp(),
        "bug: the walk reached below the account's oldest workout"
    );
    assert_eq!(
        served_ids(&resources, user_id, tenant).await,
        vec![OLD_RUN, MID_RIDE, RECENT_RIDE],
        "bug: the walked history is served through the grant's window"
    );
    // The wellness half read the session over the year before the grant's
    // first day, each day once, and never the grant's day or after: the
    // push holds that day on.
    let today = Utc::now().date_naive();
    let summary_days: Vec<NaiveDate> =
        calls_matching(&calls, &format!("summary {SIGNED_IN_SESSION} "))
            .iter()
            .map(|call| {
                let date = call.rsplit(' ').next().unwrap_or_default();
                NaiveDate::parse_from_str(date, "%Y-%m-%d")
                    .unwrap_or_else(|e| panic!("summary read of a day: {call}: {e}"))
            })
            .collect();
    let long_before = today - ChronoDuration::days(200);
    assert!(
        summary_days.contains(&long_before),
        "bug: the walk reads wellness far older than the sync's two weeks"
    );
    assert!(
        summary_days.iter().all(|day| *day < today),
        "bug: the walk never reads a day the push holds"
    );
    assert_eq!(
        summary_days.len(),
        365,
        "bug: a year of days before the grant, each read once"
    );
    assert_eq!(
        summary_days.iter().min().copied(),
        Some(today - ChronoDuration::days(365)),
        "bug: the read starts a year before the grant's first day"
    );
    let mornings = resources
        .common
        .repos
        .recovery
        .get_recovery_metrics(
            user_id,
            &tenant,
            Utc::now() - ChronoDuration::days(400),
            Utc::now() + ChronoDuration::days(1),
        )
        .await
        .unwrap();
    assert!(
        !mornings.is_empty(),
        "bug: the mornings before the grant are copied"
    );
    assert!(
        mornings.iter().any(|morning| morning.date == long_before),
        "bug: a morning 200 days before the grant is copied: {:?}",
        mornings.iter().map(|m| m.date).min()
    );
    assert!(
        mornings.iter().all(|morning| morning.date < today),
        "bug: the walk never writes a day the push holds: {:?}",
        mornings.iter().map(|m| m.date).collect::<Vec<_>>()
    );

    assert!(
        session_token_held(&resources, user_id, tenant).await,
        "bug: the walked session's token row stays for the next walk (B1)"
    );
    let mut held = connections(&resources, user_id, tenant).await;
    held.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        held,
        vec![
            (oauth_providers::GARMIN.to_owned(), ConnectionStatus::Active),
            (
                oauth_providers::SCIOTTE_GARMIN.to_owned(),
                ConnectionStatus::Active
            ),
        ],
        "bug: the session's connection stays beside the grant (B1)"
    );
    assert_eq!(
        calls_matching(&calls, &format!("drop {SIGNED_IN_SESSION}")),
        Vec::<String>::new(),
        "bug: the scraper is never told to drop a walked session"
    );
    assert_eq!(
        notification_types(&resources, user_id, tenant).await,
        Vec::<String>::new(),
        "the walk is told to nobody"
    );
    assert_eq!(
        garmin_history(&resources, &token).await,
        json!({
            "state": "imported",
            "sign_in_target": oauth_providers::SCIOTTE_GARMIN,
            "archive_source": offered_garmin_archive(&resources),
        }),
        "bug: a walked history reads imported"
    );
}

/// The resting heart rate every morning the push stores carries, apart from
/// the one the usr/pwd session's summaries answer.
const PUSHED_RESTING_HR: u32 = 41;

/// Store, under the Garmin API's own data source as the push ingest files
/// them, one morning on each of `days`.
async fn push_mornings(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    days: &[NaiveDate],
) -> String {
    let repos = &resources.common.repos;
    let source = repos
        .data_sources
        .upsert_data_source(
            &tenant,
            &DataSource {
                id: String::new(),
                user_id: user_id.to_string(),
                provider: oauth_providers::GARMIN.to_owned(),
                device_model: None,
                software_version: None,
                // The push ingest's marker on its data source.
                source: Some(oauth_providers::GARMIN.to_owned()),
                device_type: DeviceType::Watch,
                original_source_name: Some("Garmin Connect".to_owned()),
            },
        )
        .await
        .unwrap();
    for day in days {
        repos
            .recovery
            .upsert_recovery_metrics(
                &tenant,
                &StoredRecoveryMetrics {
                    id: format!("pushed-{day}"),
                    user_id: user_id.to_string(),
                    data_source_id: source.clone(),
                    date: *day,
                    recovery_score: None,
                    readiness_score: None,
                    hrv_ms: None,
                    hrv_rmssd: None,
                    resting_heart_rate: Some(PUSHED_RESTING_HR),
                    stress_score: None,
                    body_battery: None,
                    spo2: None,
                    respiratory_rate: None,
                    skin_temp_deviation: None,
                    daily_strain: None,
                    athlete_note: None,
                    source_name: oauth_providers::GARMIN.to_owned(),
                    recorded_at: Utc::now(),
                },
                WellnessWriteMode::FullDay,
                Utc::now(),
            )
            .await
            .unwrap();
    }
    source
}

/// The grant was reconnected after its pushes stored days: the connection's
/// `connected_at` is today, later than what the push holds. The walk's
/// wellness half still stops before the first day the push stored, and every
/// pushed morning is left as the push wrote it.
#[tokio::test]
async fn a_reconnected_grants_pushed_days_are_never_overwritten_by_the_walk() {
    let (scraper, calls) = spawn_scraper(false).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    let today = Utc::now().date_naive();
    let pushed_days: Vec<NaiveDate> = (3..=10)
        .map(|back| today - ChronoDuration::days(back))
        .collect();
    let first_pushed = today - ChronoDuration::days(10);
    let source = push_mornings(&resources, user_id, tenant, &pushed_days).await;
    // The reconnect: the grant's connection is (re)registered today.
    hold_grant(&resources, user_id, tenant).await;

    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        walk_ended(&resources, user_id, tenant).await,
        BackfillJobState::Walked,
        "bug: the walk reaches the end of the history"
    );

    let summary_days: Vec<NaiveDate> =
        calls_matching(&calls, &format!("summary {SIGNED_IN_SESSION} "))
            .iter()
            .map(|call| {
                let date = call.rsplit(' ').next().unwrap_or_default();
                NaiveDate::parse_from_str(date, "%Y-%m-%d")
                    .unwrap_or_else(|e| panic!("summary read of a day: {call}: {e}"))
            })
            .collect();
    assert!(
        !summary_days.is_empty(),
        "bug: the walk still reads the session's wellness before the push"
    );
    assert!(
        summary_days.iter().all(|day| *day < first_pushed),
        "bug: the walk reads a day the push already stored, though the reconnect restamped the grant: {:?}",
        summary_days.iter().max()
    );
    let mornings = resources
        .common
        .repos
        .recovery
        .get_recovery_metrics(
            user_id,
            &tenant,
            Utc::now() - ChronoDuration::days(400),
            Utc::now() + ChronoDuration::days(1),
        )
        .await
        .unwrap();
    for day in &pushed_days {
        let morning = mornings
            .iter()
            .find(|morning| morning.date == *day)
            .unwrap_or_else(|| panic!("bug: the pushed morning of {day} is gone"));
        assert_eq!(
            (morning.data_source_id.as_str(), morning.resting_heart_rate),
            (source.as_str(), Some(PUSHED_RESTING_HR)),
            "bug: the walk overwrote the pushed morning of {day}"
        );
    }
    assert!(
        mornings.iter().any(|morning| morning.date < first_pushed),
        "bug: the mornings before the push are still copied"
    );
}

/// The Garmin web session dies mid-walk: the walk ends incomplete, which
/// the card reads, and the athlete is told nothing — no notice, the grant
/// still Connected. The dead session is kept (B1) but flagged without a
/// word, so the athlete's next sign-in signs in again instead of answering
/// from it, and that sign-in's walk lands.
#[tokio::test]
async fn a_session_expiring_mid_walk_reads_incomplete_and_sends_nothing() {
    let switch = ScraperSwitch::default();
    switch.expired.store(true, Ordering::SeqCst);
    let (scraper, calls) = spawn_switchable_scraper(switch.clone()).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    assert_eq!(
        garmin_history(&resources, &token).await["state"],
        "needed",
        "a grant with nothing from before it needs its history"
    );

    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        walk_ended(&resources, user_id, tenant).await,
        BackfillJobState::Incomplete,
        "bug: the job says the walk did not finish"
    );

    assert!(
        !calls_matching(&calls, &format!("activities {SIGNED_IN_SESSION}")).is_empty(),
        "the walk read the session"
    );
    assert_eq!(
        garmin_history(&resources, &token).await,
        json!({
            "state": "incomplete",
            "sign_in_target": oauth_providers::SCIOTTE_GARMIN,
            "archive_source": offered_garmin_archive(&resources),
        }),
        "bug: a walk ended before its history landed reads incomplete"
    );
    assert_eq!(
        reconnect_notices(&resources, user_id, tenant).await,
        0,
        "bug: a history session's expiry is never a reconnect"
    );
    let mut held = connections(&resources, user_id, tenant).await;
    held.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        held,
        vec![
            (oauth_providers::GARMIN.to_owned(), ConnectionStatus::Active),
            (
                oauth_providers::SCIOTTE_GARMIN.to_owned(),
                ConnectionStatus::NeedsReauth
            ),
        ],
        "bug: the grant stands, and the dead session is kept, flagged so no stored-session reuse answers from it"
    );
    assert_eq!(
        rows_under(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        0,
        "bug: nothing landed under the session's key"
    );
    assert_eq!(
        notification_types(&resources, user_id, tenant).await,
        Vec::<String>::new(),
        "bug: a walk that ends incomplete is told to nobody"
    );

    // "Continue": the athlete signs in again on a session that lives, and
    // that sign-in's walk replaces the incomplete row and lands.
    switch.expired.store(false, Ordering::SeqCst);
    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        walk_ended(&resources, user_id, tenant).await,
        BackfillJobState::Walked,
        "bug: an ended walk never blocks the next one"
    );
    assert_eq!(
        rows_under(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        3,
        "bug: the copy lands under the session's key"
    );
    assert_eq!(
        garmin_history(&resources, &token).await["state"],
        "imported",
        "bug: a landed copy reads imported"
    );
}

/// A Garmin athlete with no grant signs in by username and password as
/// before: the session is their ongoing source, so no walk starts at the
/// sign-in, nothing is retired, and the card has no history to offer.
#[tokio::test]
async fn a_usr_pwd_only_garmin_sign_in_is_untouched() {
    let (scraper, calls) = spawn_scraper(false).await;
    let resources = server(common::test_server_config_with_sciotte_usr_pwd_garmin(
        &scraper,
    ))
    .await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;

    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(status, 200, "{body}");
    // The pre-fetch a sign-in starts is the session's own read.
    let deadline = Instant::now() + IMPORT_DEADLINE;
    while calls_matching(&calls, "activities").is_empty() {
        assert!(Instant::now() < deadline, "the pre-fetch never read");
        sleep(Duration::from_millis(50)).await;
    }
    sleep(Duration::from_millis(500)).await;

    assert!(
        session_token_held(&resources, user_id, tenant).await,
        "bug: a usr/pwd-only athlete keeps their session"
    );
    assert_eq!(
        connections(&resources, user_id, tenant).await,
        vec![(
            oauth_providers::SCIOTTE_GARMIN.to_owned(),
            ConnectionStatus::Active
        )],
        "bug: a session with no grant beside it stays"
    );
    assert_eq!(
        job_state(&resources, user_id, tenant).await,
        None,
        "bug: no job row is left behind"
    );
    assert_eq!(
        resources
            .common
            .repos
            .activity_backfill_jobs
            .job_progress(
                user_id,
                tenant,
                oauth_providers::SCIOTTE_GARMIN,
                BackfillJobKind::HistoryWalk
            )
            .await
            .unwrap(),
        None,
        "bug: a sign-in with no grant beside it walks nothing at the sign-in"
    );
    assert_eq!(
        calls_matching(&calls, "drop"),
        Vec::<String>::new(),
        "bug: nothing drops the session"
    );
    assert_eq!(
        garmin_history(&resources, &token).await,
        json!({
            "state": "not_applicable",
            "sign_in_target": null,
            "archive_source": offered_garmin_archive(&resources),
        }),
        "bug: a card no grant serves has no history to bring in"
    );
}

/// The job row a history walk of the session records.
pub(crate) fn job_row(
    user_id: Uuid,
    tenant: TenantId,
    created_at_ms: i64,
) -> ActivityBackfillJobRow {
    ActivityBackfillJobRow {
        id: Uuid::new_v4().to_string(),
        tenant_id: tenant,
        user_id,
        provider: oauth_providers::SCIOTTE_GARMIN.to_owned(),
        after_ts: Some(WHOLE_ACCOUNT_AFTER),
        before_ts: None,
        fetch_limit: Some(2_000),
        conversation_id: None,
        created_at_ms,
        leased_until_ms: 0,
        attempts: 1,
        lease_token: None,
        kind: BackfillJobKind::HistoryWalk,
        progress: JobProgress::default(),
        turn_id: None,
        asked_message_id: None,
    }
}

/// An instance died holding a walk: the resume sweep re-runs it on the
/// session the row names — never the grant routing would pick — step after
/// step, and the session stays when the walk ends (B1).
#[tokio::test]
async fn a_resumed_walk_reads_the_session_and_keeps_it() {
    let (scraper, calls) = spawn_scraper(false).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;
    let long_ago = Utc::now().timestamp_millis() - 600_000;
    assert!(
        resources
            .common
            .repos
            .activity_backfill_jobs
            .record_backfill_job(&job_row(user_id, tenant, long_ago))
            .await
            .unwrap(),
        "bug: the ledger records the walk"
    );

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let resumed = resume_backfill_jobs(runtime).await.unwrap();
    assert_eq!(resumed, 1, "bug: the sweep resumes the owed walk");
    assert_eq!(
        walk_ended(&resources, user_id, tenant).await,
        BackfillJobState::Walked,
        "bug: the resumed walk reaches the end of the history"
    );

    assert_eq!(
        calls_matching(&calls, &format!("activities {LIVE_SESSION}")).len(),
        4,
        "bug: every step of the resumed walk reads the session it names"
    );
    assert!(
        calls_matching(&calls, "activities")
            .iter()
            .all(|call| call.starts_with(&format!("activities {LIVE_SESSION}"))),
        "bug: no step reads anything but the session"
    );
    assert_eq!(
        rows_under(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        3,
        "bug: the resumed walk lands under the session's key"
    );
    assert!(
        session_token_held(&resources, user_id, tenant).await,
        "bug: the walked session stays (B1)"
    );
}

/// The ledger keeps an ended walk's row out of every sweep: an incomplete
/// row is neither claimed nor reaped, however old.
#[tokio::test]
async fn an_incomplete_walk_row_is_never_claimed_or_reaped() {
    let resources = server(common::test_server_config()).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    let ledger = &resources.common.repos.activity_backfill_jobs;
    let long_ago = Utc::now().timestamp_millis() - 600_000;
    assert!(
        ledger
            .record_backfill_job(&job_row(user_id, tenant, long_ago))
            .await
            .unwrap(),
        "bug: the ledger records the walk"
    );
    let now_ms = Utc::now().timestamp_millis();
    let claim = BackfillJobClaim {
        now_ms,
        queued_older_than_ms: 0,
        lease_ms: 60_000,
        max_attempts: 3,
        limit: 10,
    };
    let claimed = ledger.claim_stale_backfill_jobs(claim).await.unwrap();
    let mine = claimed
        .into_iter()
        .find(|row| row.user_id == user_id)
        .expect("an owed row is claimed");
    let lease = JobLease {
        id: mine.id.clone(),
        token: mine.lease_token.clone().unwrap(),
    };
    assert!(
        ledger.end_backfill_job_incomplete(&lease).await.unwrap(),
        "bug: the lease holder ends its job incomplete"
    );
    assert_eq!(
        job_state(&resources, user_id, tenant).await,
        Some(BackfillJobState::Incomplete),
        "bug: the ended walk's row is kept as incomplete"
    );

    let later = BackfillJobClaim {
        now_ms: now_ms + 3_600_000,
        ..claim
    };
    assert!(
        !ledger
            .claim_stale_backfill_jobs(later)
            .await
            .unwrap()
            .iter()
            .any(|row| row.user_id == user_id),
        "bug: an ended walk is never run again by the sweep"
    );
    ledger
        .reap_exhausted_backfill_jobs(now_ms + 3_600_000, 0)
        .await
        .unwrap();
    assert_eq!(
        job_state(&resources, user_id, tenant).await,
        Some(BackfillJobState::Incomplete),
        "bug: the reaper keeps what the card reads"
    );
    assert!(
        ledger
            .record_backfill_job(&job_row(user_id, tenant, now_ms))
            .await
            .unwrap(),
        "bug: a new walk replaces the ended row"
    );
    assert_eq!(
        job_state(&resources, user_id, tenant).await,
        Some(BackfillJobState::Owed),
        "bug: a new ask replaces the ended row"
    );
    assert!(
        !ledger
            .record_backfill_job(&job_row(user_id, tenant, now_ms))
            .await
            .unwrap(),
        "an owed walk is the same job: a second ask records nothing"
    );
}

/// A server whose Garmin OAuth reaches the dev fixture's stand-in and whose
/// scraper is `scraper`.
pub(crate) async fn server_with_garmin_oauth(scraper: &str) -> Arc<ServerContext> {
    let fixture_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fixture_base = format!("http://{}", fixture_listener.local_addr().unwrap());
    let mut config = common::test_server_config_with_sciotte(scraper);
    common::set_server_app(
        &mut config,
        "garmin",
        "garmin-history-client",
        "garmin-secret",
    );
    common::set_provider_endpoints(
        &mut config,
        "garmin",
        ProviderEndpointOverride {
            auth_url: Some(format!("{fixture_base}/oauth2Confirm")),
            token_url: Some(format!("{fixture_base}/di-oauth2-service/oauth/token")),
            api_base_url: Some(fixture_base.clone()),
            revoke_url: Some(format!("{fixture_base}/user/registration")),
            ..ProviderEndpointOverride::default()
        },
    );
    config.external_services.garmin_api.base_url = format!("{fixture_base}/wellness-api/rest");
    let fixture = GarminFixture::try_new(GarminFixtureConfig {
        push: None,
        files_base: format!("{fixture_base}/wellness-api/rest"),
        auto_push_after: None,
    })
    .unwrap();
    let fixture_app = garmin::router(fixture);
    tokio::spawn(async move {
        axum::serve(fixture_listener, fixture_app).await.unwrap();
    });
    server(config).await
}

/// Connect the athlete's Garmin through the fixture as a browser would,
/// landing on this server's OAuth callback route.
pub(crate) async fn connect_garmin_oauth(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) {
    let service = OAuthService::new(resources.data(), resources.common.config.clone());
    let authorization = service
        .get_auth_url(user_id, tenant, "garmin", AuthUrlOptions::default())
        .await
        .expect("a Garmin authorization URL");
    let browser = reqwest::Client::builder()
        .redirect(Policy::none())
        .build()
        .unwrap();
    let consent = browser
        .get(&authorization.authorization_url)
        .send()
        .await
        .unwrap();
    let location = consent
        .headers()
        .get(LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(Url::parse)
        .expect("the consent page redirects")
        .unwrap();
    let query = location.query().expect("the redirect carries its code");
    let resp = AxumTestRequest::get(&format!("/api/oauth/callback/garmin?{query}"))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    assert!(
        resp.status() < 400,
        "the callback lands: {} {}",
        resp.status(),
        resp.text()
    );
    assert!(
        resources
            .common
            .repos
            .oauth_tokens
            .get_token(user_id, tenant, "garmin")
            .await
            .unwrap()
            .is_some(),
        "bug: the grant's token row stays"
    );
}

/// An athlete who signed in by username and password before moving to the
/// grant: the callback finds their live session beside the new grant and
/// walks the history through it, keeping it for the next walk (B1).
#[tokio::test]
async fn the_oauth_callback_walks_through_a_live_session_and_keeps_it() {
    let (scraper, calls) = spawn_scraper(false).await;
    let resources = server_with_garmin_oauth(&scraper).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;

    connect_garmin_oauth(&resources, user_id, tenant).await;
    walk_started(&resources, user_id, tenant).await;
    assert_eq!(
        walk_ended(&resources, user_id, tenant).await,
        BackfillJobState::Walked,
        "bug: the callback's walk reaches the end of the history"
    );

    let reads = calls_matching(&calls, &format!("activities {LIVE_SESSION}"));
    assert_eq!(
        reads.len(),
        4,
        "bug: the callback walks the live session, step by step: {reads:?}"
    );
    assert_eq!(
        rows_under(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        3,
        "bug: the walk lands under the session's key"
    );
    assert!(
        connections(&resources, user_id, tenant)
            .await
            .iter()
            .any(|(provider, _)| provider == oauth_providers::SCIOTTE_GARMIN),
        "bug: the walked session's connection stays (B1)"
    );
    assert_eq!(
        reconnect_notices(&resources, user_id, tenant).await,
        0,
        "bug: a walk sends no reconnect notice"
    );
    assert_eq!(
        garmin_history(&resources, &token).await["state"],
        "imported",
        "bug: a walked history reads imported"
    );
}

/// Wait until the walk the callback started detached is on the ledger.
pub(crate) async fn walk_started(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId) {
    let deadline = Instant::now() + IMPORT_DEADLINE;
    while job_state(resources, user_id, tenant).await.is_none() {
        assert!(
            Instant::now() < deadline,
            "the callback never started a walk"
        );
        sleep(Duration::from_millis(50)).await;
    }
}

/// Wait until the athlete's session is retired.
async fn session_retired(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId) {
    let deadline = Instant::now() + IMPORT_DEADLINE;
    while session_token_held(resources, user_id, tenant).await {
        assert!(
            Instant::now() < deadline,
            "the dead session was never retired"
        );
        sleep(Duration::from_millis(50)).await;
    }
}

/// The callback finds a flagged session beside the new grant: it is retired
/// at once, never read, and its expiry is never told.
#[tokio::test]
async fn the_oauth_callback_retires_a_dead_session_at_once() {
    let (scraper, calls) = spawn_scraper(false).await;
    let resources = server_with_garmin_oauth(&scraper).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_session(&resources, user_id, tenant, DEAD_SESSION).await;
    resources
        .common
        .repos
        .provider_connections
        .mark_needs_reauth(
            user_id,
            tenant,
            oauth_providers::SCIOTTE_GARMIN,
            Some("session_expired"),
            Utc::now(),
        )
        .await
        .unwrap();

    connect_garmin_oauth(&resources, user_id, tenant).await;
    session_retired(&resources, user_id, tenant).await;

    assert!(
        calls_matching(&calls, &format!("activities {DEAD_SESSION}")).is_empty(),
        "bug: a dead session is retired, never read"
    );
    assert_eq!(
        job_state(&resources, user_id, tenant).await,
        None,
        "bug: no job row is left behind"
    );
    assert!(
        !connections(&resources, user_id, tenant)
            .await
            .iter()
            .any(|(provider, _)| provider == oauth_providers::SCIOTTE_GARMIN),
        "bug: the retired session leaves no connection row"
    );
    assert_eq!(
        reconnect_notices(&resources, user_id, tenant).await,
        0,
        "bug: a history session's expiry sends no reconnect notice"
    );
}

/// Retirement is no disconnect: every activity the session copied stays, a
/// non-mirror slug retires nothing, and the grant's rows are never touched.
#[tokio::test]
async fn retirement_keeps_every_copied_row_and_never_takes_a_grant() {
    let resources = server(common::test_server_config()).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;
    let ride = ActivityBuilder::new(
        MID_RIDE,
        "Copied Ride",
        SportType::Ride,
        Utc::now() - ChronoDuration::days(400),
        3_600,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .build();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(user_id, &tenant, oauth_providers::SCIOTTE_GARMIN, &[ride])
        .await
        .unwrap();

    retire_history_session(
        &resources.common.repos,
        None,
        user_id,
        tenant,
        oauth_providers::GARMIN,
    )
    .await
    .unwrap();
    assert_eq!(
        connections(&resources, user_id, tenant).await.len(),
        2,
        "bug: a slug that is no mirror retires nothing"
    );

    retire_history_session(
        &resources.common.repos,
        None,
        user_id,
        tenant,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .await
    .unwrap();
    assert_eq!(
        rows_under(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        1,
        "bug: retirement purges nothing"
    );
    assert!(
        !session_token_held(&resources, user_id, tenant).await,
        "bug: the session's token row is retired"
    );
    assert_eq!(
        connections(&resources, user_id, tenant).await,
        vec![(oauth_providers::GARMIN.to_owned(), ConnectionStatus::Active)],
        "bug: only the grant's connection is left"
    );
}

/// A Garmin token endpoint that honours every refresh, counting them.
async fn honouring_garmin_token_endpoint() -> (String, Arc<AtomicUsize>) {
    let refreshes = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&refreshes);
    let app = Router::new().route(
        "/oauth/token",
        post(move || {
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Json(json!({
                    "access_token": "garmin-access-renewed",
                    "refresh_token": "garmin-refresh-renewed",
                    "token_type": "bearer",
                    "expires_in": 86_400,
                }))
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}/oauth/token"), refreshes)
}

/// A refresh that lands proves the grant alive, never a withdrawn
/// permission back: the `permission_withdrawn` flag stands through it, while
/// a flag a refused refresh raised is cleared by the refresh that lands.
#[tokio::test]
async fn a_refresh_that_lands_never_clears_a_withdrawn_permission() {
    let (token_url, refreshes) = honouring_garmin_token_endpoint().await;
    let mut config = common::test_server_config();
    common::set_server_app(
        &mut config,
        "garmin",
        "garmin-refresh-client",
        "garmin-secret",
    );
    common::set_provider_endpoints(
        &mut config,
        "garmin",
        ProviderEndpointOverride {
            token_url: Some(token_url),
            ..ProviderEndpointOverride::default()
        },
    );
    let resources = server(config).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();

    for (reason, stands) in [(PERMISSION_WITHDRAWN, true), ("grant_refused", false)] {
        let (user_id, _user, tenant, _token) = athlete(&resources).await;
        hold_grant(&resources, user_id, tenant).await;
        let mut expired = resources
            .common
            .repos
            .oauth_tokens
            .get_token(user_id, tenant, oauth_providers::GARMIN)
            .await
            .unwrap()
            .unwrap();
        expired.expires_at = Some(Utc::now() - ChronoDuration::hours(1));
        resources
            .common
            .repos
            .oauth_tokens
            .upsert_token(&expired)
            .await
            .unwrap();
        resources
            .common
            .repos
            .provider_connections
            .mark_needs_reauth(
                user_id,
                tenant,
                oauth_providers::GARMIN,
                Some(reason),
                Utc::now(),
            )
            .await
            .unwrap();
        let before = refreshes.load(Ordering::SeqCst);

        let token = AuthService::new(Arc::clone(&runtime))
            .get_valid_token(user_id, oauth_providers::GARMIN, Some(&tenant.to_string()))
            .await
            .unwrap();
        assert!(token.is_some(), "the refresh lands for {reason}");
        assert_eq!(
            refreshes.load(Ordering::SeqCst),
            before + 1,
            "bug: the daily check refreshes the due grant once"
        );
        let status = connections(&resources, user_id, tenant)
            .await
            .into_iter()
            .find(|(provider, _)| provider == oauth_providers::GARMIN)
            .unwrap()
            .1;
        if stands {
            assert_eq!(
                status,
                ConnectionStatus::NeedsReauth,
                "bug: a refresh never clears a withdrawn permission"
            );
        } else {
            assert_eq!(
                status,
                ConnectionStatus::Active,
                "a refresh that lands clears a refused-refresh flag"
            );
        }
    }
}
