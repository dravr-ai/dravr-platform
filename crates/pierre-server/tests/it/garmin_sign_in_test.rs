// ABOUTME: A usr/pwd Garmin sign-in, beside a Garmin grant or alone, keeps its session as the provider's primary source
// ABOUTME: Pins that nothing retires a session, that history a retired session stored is still served, and the grant's flags

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin sign-ins under sciotte first (carnet#1494).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// A usr/pwd Garmin session is the provider's primary source, and a Garmin
// grant beside it its fallback. A sign-in, with or without a grant beside
// it, is warmed like any other and never retired; a Garmin OAuth connect
// leaves the session standing beside the new grant, live or flagged. The
// history the retired one-time import (carnet#1386) stored under the
// session's key, for athletes whose session it retired, is still served
// through the grant's window.
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
use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, Utc};
use pierre_config::environment::{ProviderEndpointOverride, ServerConfig};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{
    Activity, ActivityBuilder, ConnectionStatus, ConnectionType, SportType, TenantId, User,
    UserOAuthToken, PERMISSION_WITHDRAWN,
};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_dev_fixture::garmin::{self, GarminFixture, GarminFixtureConfig};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_notifications::{NotificationService, TenantId as CommereTenantId};
use pierre_providers::core::ActivityQueryParams;
use pierre_routes_auth::AuthRoutes;
use pierre_services::oauth_flow::{AuthUrlOptions, OAuthService};
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

/// The three workouts the stand-in's account holds, oldest first.
const OLD_RUN: &str = "7000000001";
const MID_RIDE: &str = "7000000002";
const RECENT_RIDE: &str = "7000000003";

/// How long a detached read gets to land before the test gives up.
pub(crate) const READ_DEADLINE: Duration = Duration::from_secs(20);

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

/// A stand-in for the scraper service over a Garmin account holding a 2012
/// run, a 2019 ride and a ride three days old. `expired` makes every read of
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
    let recent = (Utc::now() - ChronoDuration::days(3)).to_rfc3339();
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
                    let recent = recent.clone();
                    async move {
                        let session = session_header(&headers);
                        scraper.calls.lock().unwrap().push(format!(
                            "activities {session} {}",
                            query.unwrap_or_default()
                        ));
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
                        (
                            StatusCode::OK,
                            Json(json!({
                                "count": 3,
                                "activities": [
                                    workout(RECENT_RIDE, "ride", &recent),
                                    workout(MID_RIDE, "ride", "2019-07-14T07:30:00Z"),
                                    workout(OLD_RUN, "run", "2012-05-06T08:00:00Z"),
                                ],
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
/// database, so the notices a test expects can be counted.
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

/// The athlete's cached activities under `key`.
pub(crate) async fn rows_under(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    key: &str,
) -> usize {
    resources
        .common
        .repos
        .activity_cache
        .get_cached_activities(
            user_id,
            &tenant,
            Some(key),
            DateTime::<Utc>::UNIX_EPOCH,
            Utc::now() + ChronoDuration::days(1),
            1_000,
        )
        .await
        .unwrap()
        .len()
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

/// Wait until the session's pre-fetch has read the account, then a moment
/// more for anything a sign-in might start after it.
async fn prefetch_read(calls: &Calls) {
    let deadline = Instant::now() + READ_DEADLINE;
    while calls_matching(calls, "activities").is_empty() {
        assert!(Instant::now() < deadline, "the pre-fetch never read");
        sleep(Duration::from_millis(50)).await;
    }
    sleep(Duration::from_millis(500)).await;
}

/// A Garmin athlete with no grant signs in by username and password: the
/// session is their ongoing source, nothing retires it, and the card has no
/// history to offer.
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
    prefetch_read(&calls).await;

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

/// A Garmin grant holder signs in with their username and password: the
/// session becomes the provider's primary source (carnet#1494). It is warmed
/// like any sign-in, by its own read, and nothing retires it: its token and
/// connection stay beside the live grant, the scraper is never told to drop
/// it, nobody is told anything, and the card reads the session, which brings
/// its own history in.
#[tokio::test]
async fn a_grant_holders_sign_in_keeps_the_session_as_the_primary() {
    let (scraper, calls) = spawn_scraper(false).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;

    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(status, 200, "{body}");
    prefetch_read(&calls).await;

    assert!(
        !calls_matching(&calls, &format!("activities {SIGNED_IN_SESSION}")).is_empty(),
        "bug: the sign-in warms the session by its own read"
    );
    assert!(
        session_token_held(&resources, user_id, tenant).await,
        "bug: a session beside a grant is never retired"
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
        "bug: the session stands beside the live grant"
    );
    assert_eq!(
        calls_matching(&calls, "drop"),
        Vec::<String>::new(),
        "bug: nothing drops the session"
    );
    assert_eq!(
        reconnect_notices(&resources, user_id, tenant).await,
        0,
        "bug: a sign-in beside a live grant sends no reconnect"
    );
    assert_eq!(
        garmin_history(&resources, &token).await["state"],
        "not_applicable",
        "bug: the session serves the card and reads its own history"
    );
}

/// An athlete whose session the retired one-time import (carnet#1386)
/// already retired holds a Garmin grant alone, and the rows that session
/// stored under its key. The grant's window still serves them
/// (`history_keys(garmin)`), merged with what the grant pushed, and the card
/// reads that history as imported.
#[tokio::test]
async fn the_history_a_retired_session_stored_is_still_served_through_the_grant() {
    let resources = server(common::test_server_config()).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    let stored = |id: &str, days: i64, key: &str| {
        ActivityBuilder::new(
            id,
            "Garmin ride",
            SportType::Ride,
            Utc::now() - ChronoDuration::days(days),
            3_600,
            key,
        )
        .build()
    };
    let cache = &resources.common.repos.activity_cache;
    cache
        .upsert_activities(
            user_id,
            &tenant,
            oauth_providers::SCIOTTE_GARMIN,
            &[stored(OLD_RUN, 20, oauth_providers::SCIOTTE_GARMIN)],
        )
        .await
        .unwrap();
    cache
        .upsert_activities(
            user_id,
            &tenant,
            oauth_providers::GARMIN,
            &[stored(RECENT_RIDE, 2, oauth_providers::GARMIN)],
        )
        .await
        .unwrap();

    assert_eq!(
        served_ids(&resources, user_id, tenant).await,
        vec![OLD_RUN, RECENT_RIDE],
        "bug: the grant's window serves what the retired session stored beside its pushes"
    );
    assert_eq!(
        garmin_history(&resources, &token).await,
        json!({
            "state": "imported",
            "sign_in_target": oauth_providers::SCIOTTE_GARMIN,
            "archive_source": offered_garmin_archive(&resources),
        }),
        "bug: what the retired session stored reads as the card's history"
    );
}

/// What the grant pushed itself starts the day it connected, so it never
/// stands for the history before it: the card still offers the sign-in.
#[tokio::test]
async fn the_grants_own_pushes_leave_the_history_needed() {
    let resources = server(common::test_server_config()).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    let pushed = ActivityBuilder::new(
        "7000000010",
        "Pushed ride",
        SportType::Ride,
        Utc::now() - ChronoDuration::hours(3),
        3_600,
        oauth_providers::GARMIN,
    )
    .build();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities_synced_at(
            user_id,
            &tenant,
            oauth_providers::GARMIN,
            &[pushed],
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(
        garmin_history(&resources, &token).await["state"],
        "needed",
        "bug: a grant's own pushes read as the history before it"
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

/// An athlete who signed in by username and password connects Garmin's
/// OAuth: the callback leaves their live session standing beside the new
/// grant. The session stays the provider's primary, so nothing drops it or
/// tells the athlete anything, and the card keeps reading the session.
#[tokio::test]
async fn the_oauth_callback_keeps_a_live_session_beside_the_new_grant() {
    let (scraper, calls) = spawn_scraper(false).await;
    let resources = server_with_garmin_oauth(&scraper).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;

    connect_garmin_oauth(&resources, user_id, tenant).await;
    sleep(Duration::from_millis(500)).await;

    assert!(
        session_token_held(&resources, user_id, tenant).await,
        "bug: a new grant never retires the session beside it"
    );
    assert!(
        connections(&resources, user_id, tenant).await.contains(&(
            oauth_providers::SCIOTTE_GARMIN.to_owned(),
            ConnectionStatus::Active
        )),
        "bug: the session's connection stays active"
    );
    assert_eq!(
        calls_matching(&calls, "drop"),
        Vec::<String>::new(),
        "bug: nothing drops the session"
    );
    assert_eq!(
        reconnect_notices(&resources, user_id, tenant).await,
        0,
        "bug: a grant landing beside a live session sends no reconnect"
    );
    assert_eq!(
        garmin_history(&resources, &token).await["state"],
        "not_applicable",
        "bug: the session keeps serving the card"
    );
}

/// The callback finds a flagged session beside the new grant: it is kept as
/// it is, never read by the callback and never deleted. Only a fresh sign-in
/// revives it; the grant is its fallback meanwhile.
#[tokio::test]
async fn the_oauth_callback_keeps_a_flagged_session_beside_the_new_grant() {
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
    sleep(Duration::from_millis(500)).await;

    assert!(
        calls_matching(&calls, &format!("activities {DEAD_SESSION}")).is_empty(),
        "bug: the callback never reads the flagged session"
    );
    assert!(
        session_token_held(&resources, user_id, tenant).await,
        "bug: a new grant never deletes the session beside it"
    );
    assert!(
        connections(&resources, user_id, tenant).await.contains(&(
            oauth_providers::SCIOTTE_GARMIN.to_owned(),
            ConnectionStatus::NeedsReauth
        )),
        "bug: the flagged session stays flagged for its own sign-in"
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

/// A refused grant is the stronger state: a restored permission never
/// re-arms it, since only a flag raised for the withdrawal is the
/// withdrawal's to lift, while a refresh that proves the token alive does.
#[tokio::test]
async fn a_refused_grant_is_lifted_by_a_refresh_never_by_a_restored_permission() {
    let resources = server(common::test_server_config()).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    let connections_repo = &resources.common.repos.provider_connections;
    connections_repo
        .mark_needs_reauth(
            user_id,
            tenant,
            oauth_providers::GARMIN,
            Some("grant_refused"),
            Utc::now(),
        )
        .await
        .unwrap();

    assert!(
        !connections_repo
            .mark_active_if_flagged_for(
                user_id,
                tenant,
                oauth_providers::GARMIN,
                PERMISSION_WITHDRAWN,
            )
            .await
            .unwrap(),
        "bug: a restored permission lifts a refused grant's flag"
    );
    assert_eq!(
        connections(&resources, user_id, tenant).await,
        vec![(
            oauth_providers::GARMIN.to_owned(),
            ConnectionStatus::NeedsReauth
        )],
        "bug: the refused grant stays flagged"
    );
    assert!(
        connections_repo
            .mark_active_unless_flagged_for(
                user_id,
                tenant,
                oauth_providers::GARMIN,
                PERMISSION_WITHDRAWN,
            )
            .await
            .unwrap(),
        "bug: a refresh that proves the token alive re-arms a refused grant"
    );
}
