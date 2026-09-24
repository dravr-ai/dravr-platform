// ABOUTME: A TrainingPeaks login records whether the account trains or coaches, off the login's own request
// ABOUTME: Pins the manages_roster grant (never revoked), the coach refusal before any scrape, and the lazy discovery
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A TrainingPeaks coach account keeps no calendar of its own. Every login is
//! probed for the account's role in a task of its own, so the login answers
//! before the profile read lands; a coach account is granted `manages_roster`,
//! which nothing here ever takes back, and the workouts misfiled under it
//! before sciotte 0.14 are deleted. Once the role is known, a read of the
//! coach account's own workouts is refused in words before any scrape; a
//! connection made before roles were recorded learns its role from the
//! scraper's refusal, or from a reused login.
//! A login reuses a stored session only when it was written moments before;
//! an older one is replaced by signing in with the submitted credentials.
//!
//! One test, because `DRAVR_SCIOTTE_REMOTE_URL` is process-wide: separate
//! tests in this binary would race on the scraper they point at. The scraper
//! is a loopback stand-in (a test double, per the repo's mock rule) that
//! answers by session and records every call it serves.

mod common;
mod helpers;

use std::env;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use common::{create_test_server_resources, create_test_user_with_email, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::constants::oauth::providers::provider_terms_version;
use pierre_core::constants::oauth_providers::TOKEN_TYPE_SESSION;
use pierre_core::models::{
    ActivityBuilder, ConnectionType, ProviderAccountRole, SportType, TenantId, UserOAuthToken,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::sciotte_remote::{
    sciotte_refusal, ATHLETE_REQUIRED, ENV_AUDIENCE, ENV_REMOTE_URL,
};
use pierre_routes_auth::AuthRoutes;
use pierre_services::oauth_flow::OAuthService;
use pierre_services::provider_revocation::DisconnectReason;
use pierre_tool_runtime::activity_fetch::fetch_provider_head;
use pierre_tool_runtime::protocol::{auth_required_provider, AuthService};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::time::sleep;
use uuid::Uuid;

/// The backend TrainingPeaks' exposure notice guards.
const TP_NOTICE_BACKEND: &str = "sciotte_trainingpeaks";

/// TrainingPeaks' current exposure-notice version.
fn tp_terms_version() -> &'static str {
    provider_terms_version(TP_NOTICE_BACKEND).expect("TrainingPeaks carries a notice")
}

/// A coach account signed in through the login flow.
const COACH_SESSION: &str = "coach-session";
/// An athlete account signed in through the login flow.
const ATHLETE_SESSION: &str = "athlete-session";
/// A coach account connected before roles were recorded, read by a tool.
const LEGACY_READ_SESSION: &str = "legacy-coach-read";
/// A coach account connected before roles were recorded, reused by a login.
const LEGACY_LOGIN_SESSION: &str = "legacy-coach-login";
/// A session stored long before the login that replaces it.
const STALE_SESSION: &str = "stale-session";

/// How long before a login the stale session was stored: well past the window
/// in which a login reuses the stored session instead of signing in.
const STALE_SESSION_AGE: chrono::Duration = chrono::Duration::hours(1);

/// How long the stand-in takes to read a coach account's profile: longer than
/// a login may take, so a login that waited on the probe would show.
const COACH_PROFILE_DELAY: Duration = Duration::from_secs(3);

/// How long a background probe gets to land before the test gives up.
const PROBE_DEADLINE: Duration = Duration::from_secs(15);

/// The text every coach-account refusal carries: where an athlete's
/// workouts are read instead.
const GROUP_LINK_POINTER: &str = "link each athlete from a group you coach";

/// Every call the stand-in served, as `"<path> <session>"`.
type Calls = Arc<Mutex<Vec<String>>>;

fn session_header(headers: &HeaderMap) -> String {
    headers
        .get("x-session-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

fn is_coach(session: &str) -> bool {
    matches!(
        session,
        COACH_SESSION | LEGACY_READ_SESSION | LEGACY_LOGIN_SESSION
    )
}

fn session_json(session_id: &str) -> Value {
    json!({
        "session_id": session_id,
        "cookies": [],
        "created_at": "2026-09-23T12:00:00Z",
        "expires_at": null
    })
}

/// A stand-in for the scraper service: a login's email picks the account it
/// signs in to, and each read answers by the session it carries.
async fn spawn_scraper(calls: Calls) -> String {
    let app = Router::new()
        .route(
            "/auth/login-with-credentials",
            post(
                |State(calls): State<Calls>, Json(body): Json<Value>| async move {
                    let email = body["email"].as_str().unwrap_or_default();
                    calls
                        .lock()
                        .unwrap()
                        .push(format!("/auth/login-with-credentials {email}"));
                    let session = if email.starts_with("coach") {
                        COACH_SESSION
                    } else {
                        ATHLETE_SESSION
                    };
                    Json(json!({
                        "status": "authenticated",
                        "session_id": session,
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
            "/auth/import-session",
            post(
                |State(calls): State<Calls>, Json(body): Json<Value>| async move {
                    let session = body["session"]["session_id"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned();
                    calls
                        .lock()
                        .unwrap()
                        .push(format!("/auth/import-session {session}"));
                    Json(json!({ "session_id": session }))
                },
            ),
        )
        .route(
            "/api/athlete",
            get(
                |State(calls): State<Calls>, headers: HeaderMap| async move {
                    let session = session_header(&headers);
                    calls
                        .lock()
                        .unwrap()
                        .push(format!("/api/athlete {session}"));
                    if is_coach(&session) {
                        sleep(COACH_PROFILE_DELAY).await;
                        return Json(json!({
                            "id": "900101",
                            "role": "coach",
                            "coached_athletes": [
                                { "id": "900001", "display_name": "Alex Athlete" }
                            ],
                            "display_name": "Casey Coach"
                        }));
                    }
                    Json(json!({
                        "id": "900001",
                        "role": "athlete",
                        "display_name": "Alex Athlete"
                    }))
                },
            ),
        )
        .route(
            "/api/activities",
            get(
                |State(calls): State<Calls>,
                 headers: HeaderMap,
                 RawQuery(query): RawQuery| async move {
                    let session = session_header(&headers);
                    calls
                        .lock()
                        .unwrap()
                        .push(format!("/api/activities {session}"));
                    let names_athlete = query.unwrap_or_default().contains("athlete=");
                    if is_coach(&session) && !names_athlete {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(json!({ "error": ATHLETE_REQUIRED })),
                        );
                    }
                    (
                        StatusCode::OK,
                        Json(json!({ "activities": [], "head_complete": true })),
                    )
                },
            ),
        )
        .with_state(calls);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn calls_to(calls: &Calls, path: &str, session: &str) -> usize {
    let wanted = format!("{path} {session}");
    calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| **call == wanted)
        .count()
}

async fn login(resources: &Arc<ServerContext>, token: &str, email: &str) -> (u16, Value) {
    let resp = AxumTestRequest::post("/api/providers/sciotte/login")
        .header("authorization", &format!("Bearer {token}"))
        .json(&json!({
            "email": email,
            "password": "not-a-real-password",
            "target": "trainingpeaks",
            "tos_consent": true,
        }))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    (
        resp.status(),
        serde_json::from_str(&resp.text()).unwrap_or(Value::Null),
    )
}

/// The `account_role` each provider card carries, keyed by provider.
async fn card_roles(resources: &Arc<ServerContext>, token: &str) -> Vec<(String, Value)> {
    let resp = AxumTestRequest::get("/api/providers")
        .header("authorization", &format!("Bearer {token}"))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    let body: Value = serde_json::from_str(&resp.text()).unwrap();
    body["providers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|card| {
            (
                card["provider"].as_str().unwrap().to_owned(),
                card.get("account_role").cloned().unwrap_or(Value::Null),
            )
        })
        .collect()
}

/// The recorded role of `user_id`'s own TrainingPeaks connection.
async fn recorded_role(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> Option<ProviderAccountRole> {
    resources
        .common
        .repos
        .provider_connections
        .get_for_user(user_id, Some(tenant))
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.provider == oauth_providers::SCIOTTE_TRAININGPEAKS)
        .and_then(|c| c.account_role)
}

async fn manages_roster(resources: &Arc<ServerContext>, user_id: Uuid) -> bool {
    resources
        .common
        .repos
        .users
        .get_global(user_id)
        .await
        .unwrap()
        .unwrap()
        .manages_roster
}

/// Wait for the background probe to record `role`, failing past the deadline.
async fn await_role(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    role: ProviderAccountRole,
) {
    let started = Instant::now();
    while recorded_role(resources, user_id, tenant).await != Some(role) {
        assert!(
            started.elapsed() < PROBE_DEADLINE,
            "the probe never recorded {role} for {user_id}"
        );
        sleep(Duration::from_millis(100)).await;
    }
}

/// A user with an accepted TrainingPeaks notice, their tenant and a token.
async fn account(resources: &Arc<ServerContext>, email: &str) -> (Uuid, TenantId, String) {
    let (user_id, user) = create_test_user_with_email(&resources.agent.database, email)
        .await
        .unwrap();
    resources
        .common
        .repos
        .users
        .record_provider_terms(user_id, TP_NOTICE_BACKEND, tp_terms_version())
        .await
        .unwrap();
    let tenant = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()
        .first()
        .unwrap()
        .id;
    let token = generate_test_token(resources, &user).await;
    (user_id, tenant, token)
}

/// A TrainingPeaks session stored the way a login before roles stores it:
/// the token, last written at `written_at`, and a connection with no role.
async fn store_legacy_session(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    session_id: &str,
    written_at: DateTime<Utc>,
) {
    let repos = &resources.common.repos;
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id: tenant.to_string(),
            provider: oauth_providers::SCIOTTE_TRAININGPEAKS.to_owned(),
            access_token: session_json(session_id).to_string(),
            refresh_token: None,
            token_type: TOKEN_TYPE_SESSION.to_owned(),
            expires_at: None,
            scope: None,
            provider_user_id: None,
            oauth_app_client_id: None,
            created_at: written_at,
            updated_at: written_at,
        })
        .await
        .unwrap();
    repos
        .provider_connections
        .register_connection(
            user_id,
            tenant,
            oauth_providers::SCIOTTE_TRAININGPEAKS,
            &ConnectionType::Manual,
            None,
        )
        .await
        .unwrap();
}

async fn cached_trainingpeaks_rows(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> usize {
    resources
        .common
        .repos
        .activity_cache
        .get_cached_activities(
            user_id,
            &tenant,
            Some(oauth_providers::SCIOTTE_TRAININGPEAKS),
            Utc::now() - chrono::Duration::days(365),
            Utc::now(),
            100,
        )
        .await
        .unwrap()
        .len()
}

/// A coach login answers before the profile read lands, then the probe
/// grants `manages_roster`, records the role on the card, deletes the
/// workouts misfiled under the account, and prefetches nothing.
async fn a_coach_login_is_granted_the_roster_off_the_request(
    resources: &Arc<ServerContext>,
    calls: &Calls,
) -> (Uuid, TenantId, String) {
    let (coach, tenant, token) = account(resources, "coach-a@example.test").await;
    // Another athlete's workout, filed under the coach by a pre-0.14 read.
    let misfiled = ActivityBuilder::new(
        "900001:4001".to_owned(),
        "Someone else's long run".to_owned(),
        SportType::Run,
        Utc::now() - chrono::Duration::days(2),
        3_600,
        "sciotte".to_owned(),
    )
    .build();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(
            coach,
            &tenant,
            oauth_providers::SCIOTTE_TRAININGPEAKS,
            &[misfiled],
        )
        .await
        .unwrap();
    assert_eq!(cached_trainingpeaks_rows(resources, coach, tenant).await, 1);

    let started = Instant::now();
    let (status, body) = login(resources, &token, "coach-a@example.test").await;
    let took = started.elapsed();
    assert_eq!(status, 200, "{body}");
    assert!(
        took < COACH_PROFILE_DELAY,
        "the login waited {took:?} — it must not wait on the profile read"
    );
    assert!(
        !manages_roster(resources, coach).await,
        "nothing is granted before the probe reads the profile"
    );
    assert_eq!(recorded_role(resources, coach, tenant).await, None);

    await_role(resources, coach, tenant, ProviderAccountRole::Coach).await;
    assert!(manages_roster(resources, coach).await);
    assert_eq!(
        cached_trainingpeaks_rows(resources, coach, tenant).await,
        0,
        "the workouts misfiled under a coach account are deleted"
    );
    assert_eq!(
        calls_to(calls, "/api/activities", COACH_SESSION),
        0,
        "a coach account has no calendar of its own to prefetch"
    );

    let roles = card_roles(resources, &token).await;
    for (provider, role) in &roles {
        let expected = if provider == oauth_providers::SCIOTTE_TRAININGPEAKS {
            json!("coach")
        } else {
            Value::Null
        };
        assert_eq!(role, &expected, "{provider} card: {roles:?}");
    }
    (coach, tenant, token)
}

/// Once the role is recorded, a read of the coach account's own workouts is
/// refused in words before any scrape, and is no reconnect prompt.
async fn a_recorded_coach_account_is_refused_before_any_scrape(
    resources: &Arc<ServerContext>,
    calls: &Calls,
    coach: Uuid,
    tenant: TenantId,
) {
    let imports_before = calls_to(calls, "/auth/import-session", COACH_SESSION);
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let tenant_str = tenant.to_string();

    let Err(refusal) = AuthService::new(Arc::clone(&runtime))
        .create_authenticated_provider("trainingpeaks", coach, Some(&tenant_str))
        .await
    else {
        panic!("a coach account's own calendar is not read");
    };
    let text = refusal.error.clone().unwrap_or_default();
    assert!(
        text.starts_with("This TrainingPeaks account is a coach account.")
            && text.contains(GROUP_LINK_POINTER),
        "{text}"
    );
    assert_eq!(
        auth_required_provider(&refusal),
        None,
        "signing in again leaves a coach account a coach account"
    );

    let error = fetch_provider_head(
        &runtime,
        "trainingpeaks",
        coach,
        &tenant_str,
        &ActivityQueryParams::with_pagination(Some(5), None),
    )
    .await
    .expect_err("the capture path is refused the same way");
    assert_eq!(error.provider_auth_required_provider(), None);
    assert!(error.message.contains(GROUP_LINK_POINTER), "{error:?}");

    assert_eq!(
        calls_to(calls, "/auth/import-session", COACH_SESSION),
        imports_before,
        "the refusal happens before the session reaches the scraper"
    );
    assert_eq!(calls_to(calls, "/api/activities", COACH_SESSION), 0);
}

/// An athlete login records the role, prefetches, and grants nothing.
async fn an_athlete_login_records_the_role_and_grants_nothing(
    resources: &Arc<ServerContext>,
    calls: &Calls,
) {
    let (athlete, tenant, token) = account(resources, "athlete-b@example.test").await;
    let (status, body) = login(resources, &token, "athlete-b@example.test").await;
    assert_eq!(status, 200, "{body}");
    await_role(resources, athlete, tenant, ProviderAccountRole::Athlete).await;
    assert!(!manages_roster(resources, athlete).await);

    let started = Instant::now();
    while calls_to(calls, "/api/activities", ATHLETE_SESSION) == 0 {
        assert!(
            started.elapsed() < PROBE_DEADLINE,
            "an athlete account's calendar is prefetched after its probe"
        );
        sleep(Duration::from_millis(100)).await;
    }
    let roles = card_roles(resources, &token).await;
    assert!(
        roles.contains(&(
            oauth_providers::SCIOTTE_TRAININGPEAKS.to_owned(),
            json!("athlete")
        )),
        "{roles:?}"
    );
}

/// A coach who later signs in with an athlete account keeps the permission.
async fn the_grant_is_never_revoked(
    resources: &Arc<ServerContext>,
    coach: Uuid,
    tenant: TenantId,
    token: &str,
) {
    OAuthService::new(resources.data(), resources.common.config.clone())
        .disconnect_provider(
            coach,
            oauth_providers::TRAININGPEAKS,
            Some(tenant.as_uuid()),
            DisconnectReason::Athlete,
        )
        .await
        .expect("a TrainingPeaks disconnect succeeds");
    let (status, body) = login(resources, token, "athlete-a@example.test").await;
    assert_eq!(status, 200, "{body}");
    await_role(resources, coach, tenant, ProviderAccountRole::Athlete).await;
    assert!(
        manages_roster(resources, coach).await,
        "an athlete account signed in later never takes the grant back"
    );
}

/// A connection made before roles were recorded learns it is a coach
/// account from the scraper's refusal of its own read, and is refused before
/// any scrape from then on.
async fn a_legacy_coach_read_is_discovered_from_the_refusal(
    resources: &Arc<ServerContext>,
    calls: &Calls,
) {
    let (coach, tenant, _token) = account(resources, "coach-c@example.test").await;
    store_legacy_session(resources, coach, tenant, LEGACY_READ_SESSION, Utc::now()).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let tenant_str = tenant.to_string();
    let params = ActivityQueryParams::with_pagination(Some(5), None);

    let error = fetch_provider_head(&runtime, "trainingpeaks", coach, &tenant_str, &params)
        .await
        .expect_err("the scraper refuses a coach account's own read");
    assert_eq!(sciotte_refusal(&error), Some(ATHLETE_REQUIRED));
    assert_eq!(error.provider_auth_required_provider(), None);
    assert!(
        error.sanitized_message().contains(GROUP_LINK_POINTER),
        "{error:?}"
    );
    assert_eq!(
        recorded_role(resources, coach, tenant).await,
        Some(ProviderAccountRole::Coach),
        "the refusal records the role before the read returns"
    );
    assert!(manages_roster(resources, coach).await);
    assert_eq!(calls_to(calls, "/api/activities", LEGACY_READ_SESSION), 1);

    fetch_provider_head(&runtime, "trainingpeaks", coach, &tenant_str, &params)
        .await
        .expect_err("the next read is refused too");
    assert_eq!(
        calls_to(calls, "/api/activities", LEGACY_READ_SESSION),
        1,
        "the next read is refused before any scrape"
    );
}

/// A login that reuses a session stored before roles were recorded probes it.
/// The session was stored moments before, so the login is answered from it
/// without signing in to the scraper again.
async fn a_reused_legacy_session_is_probed(resources: &Arc<ServerContext>, calls: &Calls) {
    let (coach, tenant, token) = account(resources, "coach-d@example.test").await;
    store_legacy_session(resources, coach, tenant, LEGACY_LOGIN_SESSION, Utc::now()).await;
    let (status, body) = login(resources, &token, "coach-d@example.test").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["short_circuit"], json!(true), "{body}");
    assert_eq!(
        calls_to(
            calls,
            "/auth/login-with-credentials",
            "coach-d@example.test"
        ),
        0,
        "a session stored moments ago answers the login without a sign-in"
    );
    await_role(resources, coach, tenant, ProviderAccountRole::Coach).await;
    assert!(manages_roster(resources, coach).await);
}

/// A session stored long before the login is not reused: the credentials the
/// user just submitted sign in again, and the session that sign-in exports
/// replaces the stored one. A TrainingPeaks session carries no expiry, so a
/// dead one is indistinguishable from a live one locally — reusing it would
/// answer "connected" while every read kept failing.
async fn a_stale_session_signs_in_again(resources: &Arc<ServerContext>, calls: &Calls) {
    let (athlete, tenant, token) = account(resources, "athlete-e@example.test").await;
    store_legacy_session(
        resources,
        athlete,
        tenant,
        STALE_SESSION,
        Utc::now() - STALE_SESSION_AGE,
    )
    .await;

    let (status, body) = login(resources, &token, "athlete-e@example.test").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({
            "status": "connected",
            "provider": oauth_providers::SCIOTTE_TRAININGPEAKS,
        }),
        "a stale stored session must not short-circuit the login"
    );
    assert_eq!(
        calls_to(
            calls,
            "/auth/login-with-credentials",
            "athlete-e@example.test"
        ),
        1,
        "the submitted credentials reach the scraper"
    );

    let stored = resources
        .common
        .repos
        .oauth_tokens
        .get_token(athlete, tenant, oauth_providers::SCIOTTE_TRAININGPEAKS)
        .await
        .unwrap()
        .expect("the login stores a session");
    let session: Value = serde_json::from_str(&stored.access_token).unwrap();
    assert_eq!(
        session["session_id"],
        json!(ATHLETE_SESSION),
        "the fresh sign-in's session replaces the stale one"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_trainingpeaks_account_role_is_read_recorded_and_acted_on() {
    let calls: Calls = Arc::new(Mutex::new(Vec::new()));
    env::set_var(ENV_REMOTE_URL, spawn_scraper(calls.clone()).await);
    env::remove_var(ENV_AUDIENCE);
    let resources = create_test_server_resources().await.unwrap();

    let (coach, tenant, token) =
        a_coach_login_is_granted_the_roster_off_the_request(&resources, &calls).await;
    a_recorded_coach_account_is_refused_before_any_scrape(&resources, &calls, coach, tenant).await;
    an_athlete_login_records_the_role_and_grants_nothing(&resources, &calls).await;
    the_grant_is_never_revoked(&resources, coach, tenant, &token).await;
    a_legacy_coach_read_is_discovered_from_the_refusal(&resources, &calls).await;
    a_reused_legacy_session_is_probed(&resources, &calls).await;
    a_stale_session_signs_in_again(&resources, &calls).await;

    env::remove_var(ENV_REMOTE_URL);
}
