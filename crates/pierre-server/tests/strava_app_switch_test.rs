// ABOUTME: carnet#503 — a Strava reconnect that lands on another shared-pool app revokes the grant on the app it leaves
// ABOUTME: Drives the real authorize URL and OAuth callback against a mocked Strava token and revocation endpoint
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The token row is one per user, tenant and provider, so a reconnect that
//! lands on a different Strava application than the one that issued the
//! stored token overwrites it, and the old grant was left authorized at
//! Strava: the athlete then counted against both applications, with nothing
//! left here to revoke the first one by. These tests go through the same two
//! calls a reconnect makes — `get_auth_url`, then `handle_callback` — against
//! a local mock of Strava's token and revocation endpoints, reached through
//! the seams production leaves unset (`PIERRE_STRAVA_TOKEN_URL` and the
//! configured revocation URL), and assert on the wire which app's grant was
//! revoked, if any.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use axum::body::{to_bytes, Body};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::{Duration, Utc};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_auth::oauth2_client::client::strava::refresh_strava_token;
#[cfg(feature = "client-chat")]
use pierre_chat_pipeline::stages::prefetch::{
    agentless_activity_window, inject_activity_refresh, prefetch_activity_context,
};
use pierre_core::errors::ErrorCode;
use pierre_core::http_client::api_client;
use pierre_core::models::{
    ActivityBuilder, ConnectionStatus, ConnectionType, SportType, TenantId, User, UserOAuthToken,
};
#[cfg(feature = "client-chat")]
use pierre_core::permissions::scopes::OAuthScope;
use pierre_database::backends::factory::Database;
use pierre_database::RepositoryRegistry;
#[cfg(feature = "health-sync")]
use pierre_enforme::error::EnformeError;
#[cfg(feature = "health-sync")]
use pierre_enforme::traits::credential_store::CredentialStore;
#[cfg(feature = "client-chat")]
use pierre_llm::ChatMessage;
use pierre_mcp_server::mcp::resources::ServerContext;
#[cfg(feature = "health-sync")]
use pierre_mcp_server::services::health_sync_refresher::install_health_sync_refresher;
use pierre_routes_auth::AuthRoutes;
use pierre_services::oauth_flow::OAuthService;
use pierre_tool_runtime::capture_sweep::{refresh_captures, RefreshOutcome, SweepBudget};
use pierre_tool_runtime::implementations::connection::mint_oauth_authorize_url;
use pierre_tool_runtime::implementations::data::GetActivitiesTool;
#[cfg(feature = "client-chat")]
use pierre_tool_runtime::implementations::data_helpers::provider_unavailable_note;
use pierre_tool_runtime::protocol::auth::{AuthService, OAuthError};
use pierre_tool_runtime::protocol::auth_required_provider;
#[cfg(feature = "client-chat")]
use pierre_tool_runtime::protocol::UniversalExecutor;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use serial_test::serial;
use tokio::net::TcpListener;
use tokio::time::sleep;
use tower::ServiceExt;
use uuid::Uuid;

/// The env-default Strava app this binary configures.
const ENV_CLIENT_ID: &str = "switch-env-client";
const ENV_CLIENT_SECRET: &str = "switch-env-secret";

/// Every pool app here shares one 30-character secret.
const POOL_SECRET: &str = "poolsecretvaluewithlength30chr";

/// The env app's seat cap, kept small so filling it is cheap.
const SEAT_CAP: u32 = 2;

/// The refresh token of the grant a reconnect replaces; revoking it is what
/// withdraws that grant at Strava.
const REPLACED_REFRESH: &str = "refresh-of-the-replaced-grant";

/// Strava's answer to a refresh with a dead refresh token (HTTP 400).
const STRAVA_DEAD_REFRESH_TOKEN: &str = r#"{"message":"Bad Request","errors":[{"resource":"RefreshToken","field":"refresh_token","code":"invalid"}]}"#;

/// Strava's documented answer to a request over its rate limit (HTTP 429). It
/// names the application without rejecting it.
const STRAVA_RATE_LIMITED: &str = r#"{"message":"Rate Limit Exceeded","errors":[{"resource":"Application","field":"rate limit","code":"exceeded"}]}"#;

/// Strava's answer to a refresh under a client it does not accept (HTTP 400).
const STRAVA_INVALID_CLIENT: &str = r#"{"message":"Bad Request","errors":[{"resource":"Application","field":"client_id","code":"invalid"}]}"#;

/// The access token another refresh of the same row stores while one is in
/// flight.
const CONCURRENT_ACCESS: &str = "access-of-a-concurrent-refresh";

/// Environment set for the duration of one test and removed after it.
struct EnvGuard {
    keys: Vec<&'static str>,
}

impl EnvGuard {
    fn set(vars: &[(&'static str, String)]) -> Self {
        for (key, value) in vars {
            env::set_var(key, value);
        }
        Self {
            keys: vars.iter().map(|(key, _)| *key).collect(),
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for key in &self.keys {
            env::remove_var(key);
        }
    }
}

/// A token another reconnect stores while this one's code exchange is in
/// flight, and where it lands.
struct RacingStore {
    repos: Arc<RepositoryRegistry>,
    token: UserOAuthToken,
}

/// What the Strava-shaped mock saw, and how it answers.
#[derive(Default)]
struct MockStrava {
    exchanges: AtomicUsize,
    /// `(authorization header, form body)` of every token request.
    token_requests: Mutex<Vec<(String, String)>>,
    /// `(authorization header, form body)` of every revocation.
    revocations: Mutex<Vec<(String, String)>>,
    /// The status and body a refresh is refused with, when set.
    refresh_rejection: Mutex<Option<(StatusCode, String)>>,
    /// Stored here during the next code exchange, when set.
    racing_store: Mutex<Option<RacingStore>>,
    /// Reconnected here during the next refresh, before it is answered, when
    /// set: the token is stored and the connection re-armed, as the OAuth
    /// callback does.
    store_during_refresh: Mutex<Option<RacingStore>>,
    /// Refreshed here during the next refresh, before it is answered, when
    /// set: another refresh of the athlete's stored row lands first, writing a
    /// new pair over the same row as a successful refresh does.
    refresh_during_refresh: Mutex<Option<(Arc<RepositoryRegistry>, Uuid, TenantId)>>,
    /// Where to look, at each revocation, for the token stored then.
    watched: Mutex<Option<(Arc<RepositoryRegistry>, Uuid, TenantId)>>,
    /// The stored token's access token at each revocation.
    stored_at_revocation: Mutex<Vec<Option<String>>>,
}

/// The authorization header of a request, empty when there is none.
fn authorization_of(headers: &HeaderMap) -> String {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

/// Stand up a mock Strava: `POST /oauth/token` answers a code exchange or a
/// refresh with a fresh token (or a refresh with the configured rejection),
/// `POST /oauth/revoke` records the revocation and confirms it.
async fn mock_strava() -> (String, Arc<MockStrava>) {
    let recorder = Arc::new(MockStrava::default());
    let token_recorder = Arc::clone(&recorder);
    let revoke_recorder = Arc::clone(&recorder);
    let app = Router::new()
        .route(
            "/oauth/token",
            post(move |headers: HeaderMap, body: String| {
                let recorder = Arc::clone(&token_recorder);
                async move {
                    recorder
                        .token_requests
                        .lock()
                        .unwrap()
                        .push((authorization_of(&headers), body.clone()));
                    if body.contains("grant_type=refresh_token") {
                        let racing = recorder.store_during_refresh.lock().unwrap().take();
                        if let Some(RacingStore { repos, token }) = racing {
                            repos.oauth_tokens.upsert_token(&token).await.unwrap();
                            let tenant = TenantId::parse_str(&token.tenant_id).unwrap();
                            repos
                                .provider_connections
                                .register_connection(
                                    token.user_id,
                                    tenant,
                                    "strava",
                                    &ConnectionType::OAuth,
                                    None,
                                )
                                .await
                                .unwrap();
                        }
                        let concurrent = recorder.refresh_during_refresh.lock().unwrap().take();
                        if let Some((repos, user_id, tenant)) = concurrent {
                            let row = repos
                                .oauth_tokens
                                .get_token(user_id, tenant, "strava")
                                .await
                                .unwrap()
                                .unwrap();
                            let landed = repos
                                .oauth_tokens
                                .refresh_token(
                                    &row,
                                    CONCURRENT_ACCESS,
                                    Some("refresh-of-a-concurrent-refresh"),
                                    Some(Utc::now() + Duration::hours(6)),
                                )
                                .await
                                .unwrap();
                            assert!(landed, "the concurrent refresh lands over the row");
                        }
                        let rejection = recorder.refresh_rejection.lock().unwrap().clone();
                        if let Some((status, body)) = rejection {
                            return (status, body).into_response();
                        }
                    } else {
                        recorder.exchanges.fetch_add(1, Ordering::SeqCst);
                        let racing = recorder.racing_store.lock().unwrap().take();
                        if let Some(RacingStore { repos, token }) = racing {
                            repos.oauth_tokens.upsert_token(&token).await.unwrap();
                        }
                    }
                    Json(json!({
                        "access_token": "access-of-the-new-grant",
                        "refresh_token": "refresh-of-the-new-grant",
                        "token_type": "Bearer",
                        "expires_in": 21_600,
                        "expires_at": (Utc::now() + Duration::hours(6)).timestamp(),
                        "athlete": { "id": 4242 }
                    }))
                    .into_response()
                }
            }),
        )
        .route(
            "/oauth/revoke",
            post(move |headers: HeaderMap, body: String| {
                let recorder = Arc::clone(&revoke_recorder);
                async move {
                    let watched = recorder.watched.lock().unwrap().clone();
                    if let Some((repos, user_id, tenant)) = watched {
                        let stored = repos
                            .oauth_tokens
                            .get_token(user_id, tenant, "strava")
                            .await
                            .unwrap()
                            .map(|token| token.access_token);
                        recorder.stored_at_revocation.lock().unwrap().push(stored);
                    }
                    recorder
                        .revocations
                        .lock()
                        .unwrap()
                        .push((authorization_of(&headers), body));
                    StatusCode::OK
                }
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (format!("http://{addr}"), recorder)
}

/// A server context whose Strava code exchange reaches the mock, and the
/// OAuth service whose revocations do. The registry reads
/// `PIERRE_STRAVA_TOKEN_URL` when the context is built, so the guard is set
/// first.
async fn service_pointed_at(base: &str) -> (Arc<ServerContext>, OAuthService, EnvGuard) {
    let guard = EnvGuard::set(&[
        ("PIERRE_STRAVA_TOKEN_URL", format!("{base}/oauth/token")),
        ("STRAVA_CLIENT_ID", ENV_CLIENT_ID.to_owned()),
        ("STRAVA_CLIENT_SECRET", ENV_CLIENT_SECRET.to_owned()),
        ("STRAVA_OAUTH_SEAT_CAP", SEAT_CAP.to_string()),
    ]);
    let resources = common::create_test_server_resources().await.unwrap();
    let mut config = (*resources.common.config).clone();
    config.external_services.strava_api.revoke_url = format!("{base}/oauth/revoke");
    let service = OAuthService::new(resources.data(), Arc::new(config));
    (resources, service, guard)
}

/// An athlete with an account and a tenant of their own.
async fn athlete(resources: &ServerContext, label: &str) -> (Uuid, TenantId) {
    let email = format!("{label}-{}@example.com", Uuid::new_v4());
    let (user_id, _, tenant_id) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .unwrap();
    (user_id, tenant_id)
}

/// Leave the state a completed OAuth callback leaves: a Strava token issued by
/// `app` (`None` = the env app) and an active connection row.
async fn connect(repos: &RepositoryRegistry, user_id: Uuid, tenant: TenantId, app: Option<&str>) {
    let token = UserOAuthToken::new(
        user_id,
        tenant.to_string(),
        "strava".to_owned(),
        "access-of-the-replaced-grant".to_owned(),
        Some(REPLACED_REFRESH.to_owned()),
        Some(Utc::now() + Duration::hours(6)),
        Some("read".to_owned()),
    )
    .with_oauth_app_client_id(app.map(str::to_owned));
    repos.oauth_tokens.upsert_token(&token).await.unwrap();
    repos
        .provider_connections
        .register_connection(user_id, tenant, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
}

/// Fill the env app to its cap with other athletes' live grants.
async fn fill_env_app(repos: &RepositoryRegistry) {
    for _ in 0..SEAT_CAP {
        let user = User::new(
            format!("filler-{}@example.com", Uuid::new_v4()),
            "argon2-hash-placeholder".to_owned(),
            None,
        );
        let user_id = repos.users.create(&user).await.unwrap();
        connect(repos, user_id, TenantId::generate(), None).await;
    }
}

/// Run a reconnect end to end — the authorize URL, then the callback that
/// consumes its state — and return the `client_id` the URL sent the athlete to.
async fn reconnect(service: &OAuthService, user_id: Uuid, tenant: TenantId) -> String {
    let authorization = service
        .get_auth_url(user_id, tenant, "strava")
        .await
        .expect("a seat remains somewhere");
    service
        .handle_callback("auth-code", &authorization.state, "strava")
        .await
        .expect("the exchange succeeds against the mock token endpoint");
    authorization
        .authorization_url
        .split(['?', '&'])
        .find_map(|pair| pair.strip_prefix("client_id="))
        .expect("the authorize URL names its client")
        .to_owned()
}

/// The stored Strava token for a user in a tenant.
async fn stored(repos: &RepositoryRegistry, user_id: Uuid, tenant: TenantId) -> UserOAuthToken {
    repos
        .oauth_tokens
        .get_token(user_id, tenant, "strava")
        .await
        .unwrap()
        .expect("a Strava token row exists")
}

/// `Basic` credentials for a client, as a revocation authenticates.
fn basic(client_id: &str, client_secret: &str) -> String {
    format!(
        "Basic {}",
        BASE64.encode(format!("{client_id}:{client_secret}"))
    )
}

/// An athlete whose env-app grant died, reconnecting while the env app is full
/// counting everyone else, is sent to the pool — and the callback revokes the
/// env-app grant under the env client before the pool token replaces it.
#[tokio::test]
#[serial]
async fn a_dead_athlete_moved_to_the_pool_has_the_env_grant_revoked_first() {
    let (base, mock) = mock_strava().await;
    let (resources, service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    repos
        .oauth_tokens
        .upsert_strava_pool_app("910001", POOL_SECRET, 5, Some("pool-a"))
        .await
        .unwrap();
    let (user_id, tenant) = athlete(&resources, "dead-env").await;
    connect(repos, user_id, tenant, None).await;
    repos
        .provider_connections
        .mark_needs_reauth(user_id, tenant, "strava", Some("invalid_grant"), Utc::now())
        .await
        .unwrap();
    fill_env_app(repos).await;

    let client_id = reconnect(&service, user_id, tenant).await;

    assert_eq!(
        client_id, "910001",
        "the env app is full, so the pool takes the athlete"
    );
    assert_eq!(
        mock.exchanges.load(Ordering::SeqCst),
        1,
        "one code exchange"
    );
    let revocations = mock.revocations.lock().unwrap().clone();
    assert_eq!(
        revocations.len(),
        1,
        "the replaced grant is revoked exactly once: {revocations:?}"
    );
    assert_eq!(
        revocations[0].0,
        basic(ENV_CLIENT_ID, ENV_CLIENT_SECRET),
        "the env app's grant is revoked as the env app"
    );
    assert!(
        revocations[0]
            .1
            .contains(&format!("token={REPLACED_REFRESH}")),
        "the revocation spends the replaced token: {}",
        revocations[0].1
    );
    let token = stored(repos, user_id, tenant).await;
    assert_eq!(token.oauth_app_client_id.as_deref(), Some("910001"));
    assert_eq!(token.access_token, "access-of-the-new-grant");
    let connection = repos
        .provider_connections
        .get_for_user(user_id, Some(tenant))
        .await
        .unwrap();
    assert_eq!(connection.len(), 1);
    assert_eq!(connection[0].status, ConnectionStatus::Active);
}

/// An athlete on a pool app an operator disabled reconnects onto the env app,
/// and the disabled app's grant is revoked under that app's own client: its
/// secret still resolves, which is what lets the grant be withdrawn.
#[tokio::test]
#[serial]
async fn a_move_off_a_disabled_pool_app_revokes_that_apps_grant() {
    let (base, mock) = mock_strava().await;
    let (resources, service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    repos
        .oauth_tokens
        .upsert_strava_pool_app("910002", POOL_SECRET, 5, Some("pool-b"))
        .await
        .unwrap();
    let (user_id, tenant) = athlete(&resources, "drained").await;
    connect(repos, user_id, tenant, Some("910002")).await;
    repos
        .oauth_tokens
        .set_strava_pool_app_enabled("910002", false)
        .await
        .unwrap();

    let client_id = reconnect(&service, user_id, tenant).await;

    assert_eq!(
        client_id, ENV_CLIENT_ID,
        "a disabled pool app is not reconnected on"
    );
    let revocations = mock.revocations.lock().unwrap().clone();
    assert_eq!(revocations.len(), 1, "{revocations:?}");
    assert_eq!(
        revocations[0].0,
        basic("910002", POOL_SECRET),
        "the pool app's grant is revoked as the pool app"
    );
    assert!(revocations[0]
        .1
        .contains(&format!("token={REPLACED_REFRESH}")));
    assert_eq!(
        stored(repos, user_id, tenant).await.oauth_app_client_id,
        None
    );
}

/// A reconnect that stays on the app that issued the stored token replaces it
/// and revokes nothing: Strava counts the athlete once per app.
#[tokio::test]
#[serial]
async fn a_reconnect_on_the_same_app_revokes_nothing() {
    let (base, mock) = mock_strava().await;
    let (resources, service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    let (user_id, tenant) = athlete(&resources, "steady").await;
    connect(repos, user_id, tenant, None).await;

    let client_id = reconnect(&service, user_id, tenant).await;

    assert_eq!(client_id, ENV_CLIENT_ID);
    assert!(
        mock.revocations.lock().unwrap().is_empty(),
        "no grant is replaced across apps"
    );
    let token = stored(repos, user_id, tenant).await;
    assert_eq!(token.access_token, "access-of-the-new-grant");
    assert_eq!(token.oauth_app_client_id, None);
}

/// A user in two tenants whose token a pool app issued in the second one
/// reconnects while active there: the authorize URL keeps them on that pool
/// app even though the env app has room, and the callback stores the new token
/// in that same tenant rather than the user's first one, so nothing is
/// replaced across apps and nothing is revoked.
#[tokio::test]
#[serial]
async fn a_second_tenant_reconnect_stays_on_its_app_and_lands_in_that_tenant() {
    let (base, mock) = mock_strava().await;
    let (resources, service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    repos
        .oauth_tokens
        .upsert_strava_pool_app("910003", POOL_SECRET, 5, Some("pool-c"))
        .await
        .unwrap();
    let (user_id, first_tenant) = athlete(&resources, "two-clubs").await;
    let (_, second_tenant) = athlete(&resources, "other-club-owner").await;
    repos
        .users
        .update_tenant_id(user_id, second_tenant)
        .await
        .unwrap();
    let memberships = repos.tenants.list_for_user(user_id).await.unwrap();
    assert_eq!(
        memberships.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![first_tenant, second_tenant],
        "the tenant the reconnect is made in is not the user's first"
    );
    connect(repos, user_id, second_tenant, Some("910003")).await;

    let client_id = reconnect(&service, user_id, second_tenant).await;

    assert_eq!(
        client_id, "910003",
        "the athlete's own pool app wins over an env app with room"
    );
    assert!(
        mock.revocations.lock().unwrap().is_empty(),
        "the reconnect replaced the token on the same app"
    );
    let token = stored(repos, user_id, second_tenant).await;
    assert_eq!(token.access_token, "access-of-the-new-grant");
    assert_eq!(token.oauth_app_client_id.as_deref(), Some("910003"));
    assert!(
        repos
            .oauth_tokens
            .get_token(user_id, first_tenant, "strava")
            .await
            .unwrap()
            .is_none(),
        "the token lands in the tenant the authorization was made in"
    );
}

/// A user whose Strava token lives in another tenant starts a first connect
/// in this one on the app that token names: Strava already counts them there.
#[tokio::test]
#[serial]
async fn a_token_in_another_tenant_names_the_app_a_new_tenant_connects_on() {
    let (base, mock) = mock_strava().await;
    let (resources, service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    repos
        .oauth_tokens
        .upsert_strava_pool_app("910004", POOL_SECRET, 5, Some("pool-d"))
        .await
        .unwrap();
    let (user_id, first_tenant) = athlete(&resources, "cross-tenant").await;
    let (_, second_tenant) = athlete(&resources, "second-club-owner").await;
    repos
        .users
        .update_tenant_id(user_id, second_tenant)
        .await
        .unwrap();
    connect(repos, user_id, first_tenant, Some("910004")).await;

    let client_id = reconnect(&service, user_id, second_tenant).await;

    assert_eq!(client_id, "910004");
    assert!(mock.revocations.lock().unwrap().is_empty());
    assert_eq!(
        stored(repos, user_id, second_tenant)
            .await
            .oauth_app_client_id
            .as_deref(),
        Some("910004")
    );
    assert_eq!(
        stored(repos, user_id, first_tenant).await.access_token,
        "access-of-the-replaced-grant",
        "the other tenant's token is untouched"
    );
}

/// A Strava token the env app or pool app `app` issued that expired an hour
/// ago, over an active connection.
async fn connect_expired(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: TenantId,
    app: Option<&str>,
) {
    connect(repos, user_id, tenant, app).await;
    let token = UserOAuthToken::new(
        user_id,
        tenant.to_string(),
        "strava".to_owned(),
        "access-of-the-replaced-grant".to_owned(),
        Some(REPLACED_REFRESH.to_owned()),
        Some(Utc::now() - Duration::hours(1)),
        Some("read".to_owned()),
    )
    .with_oauth_app_client_id(app.map(str::to_owned));
    repos.oauth_tokens.upsert_token(&token).await.unwrap();
}

/// Mark an athlete's Strava connection in `tenant` dead, as a refused refresh does.
async fn kill(repos: &RepositoryRegistry, user_id: Uuid, tenant: TenantId) {
    repos
        .provider_connections
        .mark_needs_reauth(user_id, tenant, "strava", Some("invalid_grant"), Utc::now())
        .await
        .unwrap();
}

/// A user who owns `first` and also belongs to a second tenant.
async fn athlete_in_two_tenants(
    resources: &ServerContext,
    label: &str,
) -> (Uuid, TenantId, TenantId) {
    let (user_id, first) = athlete(resources, label).await;
    let (_, second) = athlete(resources, &format!("{label}-club-owner")).await;
    resources
        .common
        .repos
        .users
        .update_tenant_id(user_id, second)
        .await
        .unwrap();
    (user_id, first, second)
}

/// An athlete whose env grant still holds its seat in one tenant connects in
/// another while the env app is full counting everyone else: Strava already
/// counts them on the env app, so they stay there and nothing is revoked,
/// where the pool would have put them on two apps.
#[tokio::test]
#[serial]
async fn a_seat_holding_grant_in_another_tenant_keeps_the_athlete_on_its_app() {
    let (base, mock) = mock_strava().await;
    let (resources, service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    repos
        .oauth_tokens
        .upsert_strava_pool_app("910011", POOL_SECRET, 5, Some("pool-k"))
        .await
        .unwrap();
    let (user_id, first, second) = athlete_in_two_tenants(&resources, "live-elsewhere").await;
    connect(repos, user_id, first, None).await;
    fill_env_app(repos).await;

    let client_id = reconnect(&service, user_id, second).await;

    assert_eq!(client_id, ENV_CLIENT_ID, "the athlete's seat-holding app");
    assert!(mock.revocations.lock().unwrap().is_empty());
    assert_eq!(
        stored(repos, user_id, second).await.oauth_app_client_id,
        None
    );
    assert_eq!(
        stored(repos, user_id, first).await.access_token,
        "access-of-the-replaced-grant",
        "the other tenant's live grant is untouched"
    );
}

/// An athlete whose env grant died in one tenant connects in another and is
/// sent to the pool: the dead grant, which Strava may still count, is revoked
/// under the env client once the pool token is stored.
#[tokio::test]
#[serial]
async fn a_move_revokes_the_athletes_unusable_grant_in_another_tenant() {
    let (base, mock) = mock_strava().await;
    let (resources, service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    repos
        .oauth_tokens
        .upsert_strava_pool_app("910012", POOL_SECRET, 5, Some("pool-l"))
        .await
        .unwrap();
    let (user_id, first, second) = athlete_in_two_tenants(&resources, "dead-elsewhere").await;
    connect(repos, user_id, first, None).await;
    kill(repos, user_id, first).await;
    fill_env_app(repos).await;

    let client_id = reconnect(&service, user_id, second).await;

    assert_eq!(
        client_id, "910012",
        "the env app is full, the grant elsewhere dead"
    );
    let revocations = mock.revocations.lock().unwrap().clone();
    assert_eq!(revocations.len(), 1, "{revocations:?}");
    assert_eq!(revocations[0].0, basic(ENV_CLIENT_ID, ENV_CLIENT_SECRET));
    assert!(revocations[0]
        .1
        .contains(&format!("token={REPLACED_REFRESH}")));
    assert_eq!(
        stored(repos, user_id, second)
            .await
            .oauth_app_client_id
            .as_deref(),
        Some("910012")
    );
}

/// The grant a switch replaces is revoked only after the new token is
/// stored: a store that failed would otherwise leave the athlete with a
/// revoked grant stored and a live one nothing holds.
#[tokio::test]
#[serial]
async fn the_replaced_grant_is_revoked_only_once_the_new_token_is_stored() {
    let (base, mock) = mock_strava().await;
    let (resources, service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    repos
        .oauth_tokens
        .upsert_strava_pool_app("910013", POOL_SECRET, 5, Some("pool-m"))
        .await
        .unwrap();
    let (user_id, tenant) = athlete(&resources, "ordered").await;
    connect(repos, user_id, tenant, None).await;
    kill(repos, user_id, tenant).await;
    fill_env_app(repos).await;
    *mock.watched.lock().unwrap() = Some((Arc::clone(repos), user_id, tenant));

    let client_id = reconnect(&service, user_id, tenant).await;

    assert_eq!(client_id, "910013");
    assert_eq!(mock.revocations.lock().unwrap().len(), 1);
    assert_eq!(
        *mock.stored_at_revocation.lock().unwrap(),
        vec![Some("access-of-the-new-grant".to_owned())],
        "the new token is durable before the old grant is withdrawn"
    );
}

/// Two reconnects for one athlete race: the other one stores its token while
/// this one's code exchange is in flight. This one stores nothing over it,
/// fails, and revokes its own fresh grant, which is on another app than the
/// token that stands.
#[tokio::test]
#[serial]
async fn a_reconnect_that_loses_a_race_stores_nothing_and_revokes_its_own_grant() {
    let (base, mock) = mock_strava().await;
    let (resources, service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    let (user_id, tenant) = athlete(&resources, "racer").await;
    connect(repos, user_id, tenant, None).await;
    kill(repos, user_id, tenant).await;
    let authorization = service
        .get_auth_url(user_id, tenant, "strava")
        .await
        .expect("the env app has room");
    let winner = UserOAuthToken::new(
        user_id,
        tenant.to_string(),
        "strava".to_owned(),
        "access-of-the-racing-grant".to_owned(),
        Some("refresh-of-the-racing-grant".to_owned()),
        Some(Utc::now() + Duration::hours(6)),
        Some("read".to_owned()),
    )
    .with_oauth_app_client_id(Some("910014".to_owned()));
    *mock.racing_store.lock().unwrap() = Some(RacingStore {
        repos: Arc::clone(repos),
        token: winner,
    });

    let error = service
        .handle_callback("auth-code", &authorization.state, "strava")
        .await
        .expect_err("the other reconnect's token landed first");

    assert_eq!(error.code, ErrorCode::ResourceLocked);
    let token = stored(repos, user_id, tenant).await;
    assert_eq!(token.access_token, "access-of-the-racing-grant");
    assert_eq!(token.oauth_app_client_id.as_deref(), Some("910014"));
    let revocations = mock.revocations.lock().unwrap().clone();
    assert_eq!(revocations.len(), 1, "{revocations:?}");
    assert_eq!(
        revocations[0].0,
        basic(ENV_CLIENT_ID, ENV_CLIENT_SECRET),
        "the losing reconnect's env grant is withdrawn as the env app"
    );
    assert!(revocations[0].1.contains("token=refresh-of-the-new-grant"));
}

/// A pool entry registered under the env app's own `client_id` is the env
/// app: an athlete moved onto it has changed no application, so the grant
/// the reconnect just made is not revoked as a replaced one.
#[tokio::test]
#[serial]
async fn a_pool_entry_under_the_env_client_id_is_the_env_app() {
    let (base, mock) = mock_strava().await;
    let (resources, service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    repos
        .oauth_tokens
        .upsert_strava_pool_app(ENV_CLIENT_ID, ENV_CLIENT_SECRET, 5, Some("env-again"))
        .await
        .unwrap();
    let (user_id, tenant) = athlete(&resources, "same-app").await;
    connect(repos, user_id, tenant, None).await;
    kill(repos, user_id, tenant).await;
    fill_env_app(repos).await;

    let client_id = reconnect(&service, user_id, tenant).await;

    assert_eq!(client_id, ENV_CLIENT_ID);
    assert!(
        mock.revocations.lock().unwrap().is_empty(),
        "one application, one grant: nothing is replaced"
    );
}

/// The authorize URL the chat reconnect and the mobile app mint names the app
/// the seat rules pick, and pins it on the state, so the callback exchanges
/// the code under that app's client.
#[tokio::test]
#[serial]
async fn a_minted_authorize_url_pins_the_pool_app_for_the_exchange() {
    let (base, mock) = mock_strava().await;
    let (resources, service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    repos
        .oauth_tokens
        .upsert_strava_pool_app("910015", POOL_SECRET, 5, Some("pool-o"))
        .await
        .unwrap();
    let (user_id, tenant) = athlete(&resources, "chat-reconnect").await;
    connect(repos, user_id, tenant, None).await;
    kill(repos, user_id, tenant).await;
    fill_env_app(repos).await;

    let runtime: &dyn ToolRuntime = resources.as_ref();
    let (url, state) = mint_oauth_authorize_url(runtime, user_id, tenant, "strava", None)
        .await
        .expect("a pool seat remains");
    assert!(
        url.contains("client_id=910015"),
        "the URL names the pool app: {url}"
    );

    service
        .handle_callback("auth-code", &state, "strava")
        .await
        .expect("the exchange succeeds under the pinned app");

    let exchanges = mock.token_requests.lock().unwrap().clone();
    assert_eq!(exchanges.len(), 1, "{exchanges:?}");
    assert!(
        exchanges[0].1.contains("client_id=910015"),
        "the code is spent under the app the URL named: {}",
        exchanges[0].1
    );
    assert_eq!(
        stored(repos, user_id, tenant)
            .await
            .oauth_app_client_id
            .as_deref(),
        Some("910015")
    );
}

/// A pool-app token that expired refreshes under that app's client, not the
/// env app's: a refresh token is spent only by the client that issued it.
#[tokio::test]
#[serial]
async fn an_expired_pool_token_refreshes_under_its_own_app() {
    let (base, mock) = mock_strava().await;
    let (resources, _service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    repos
        .oauth_tokens
        .upsert_strava_pool_app("910016", POOL_SECRET, 5, Some("pool-p"))
        .await
        .unwrap();
    let (user_id, tenant) = athlete(&resources, "pool-refresh").await;
    connect_expired(repos, user_id, tenant, Some("910016")).await;

    let auth = AuthService::new(Arc::clone(&resources) as Arc<dyn ToolRuntime>);
    let token = auth
        .get_valid_token(user_id, "strava", Some(&tenant.to_string()))
        .await
        .expect("the lookup does not error")
        .expect("the expired token is refreshed");

    assert_eq!(token.access_token, "access-of-the-new-grant");
    assert_eq!(token.oauth_app_client_id.as_deref(), Some("910016"));
    let refreshes = mock.token_requests.lock().unwrap().clone();
    assert_eq!(refreshes.len(), 1, "{refreshes:?}");
    assert!(
        refreshes[0].1.contains("client_id=910016")
            && refreshes[0]
                .1
                .contains(&format!("client_secret={POOL_SECRET}")),
        "the refresh runs as the issuing pool app: {}",
        refreshes[0].1
    );
}

/// Strava answers a refresh with a dead refresh token by naming the
/// `RefreshToken` resource, not an RFC 6749 code: that is a dead grant, so
/// the connection needs re-authorizing and the seat goes back to the pool.
#[tokio::test]
#[serial]
async fn a_strava_refresh_refused_over_a_dead_refresh_token_frees_the_seat() {
    let (base, mock) = mock_strava().await;
    let (resources, _service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    let (user_id, tenant) = athlete(&resources, "dead-refresh").await;
    connect_expired(repos, user_id, tenant, None).await;
    *mock.refresh_rejection.lock().unwrap() = Some((
        StatusCode::BAD_REQUEST,
        STRAVA_DEAD_REFRESH_TOKEN.to_owned(),
    ));

    let auth = AuthService::new(Arc::clone(&resources) as Arc<dyn ToolRuntime>);
    let token = auth
        .get_valid_token(user_id, "strava", Some(&tenant.to_string()))
        .await
        .expect("a refused refresh is not an error");
    assert!(token.is_none(), "no usable token");

    let connection = repos
        .provider_connections
        .get_for_user(user_id, Some(tenant))
        .await
        .unwrap();
    assert_eq!(connection.len(), 1);
    assert_eq!(connection[0].status, ConnectionStatus::NeedsReauth);
    let holders = repos.oauth_tokens.list_strava_seat_holders().await.unwrap();
    let holder = holders
        .iter()
        .find(|h| h.user_id == user_id)
        .expect("the token is still listed");
    assert!(!holder.counts_as_seat, "a dead grant holds no seat");
}

/// What an expiry refresh Strava did not answer with a token returns.
#[derive(Debug, PartialEq, Eq)]
enum RefreshLookup {
    /// `Ok(None)`: Strava refused the refresh, and reconnecting is the remedy.
    Refused,
    /// `Err`: the refresh failed without Strava refusing the grant.
    Failed,
}

/// Refuse the next refreshes with `status` and `body`, run one expiry refresh
/// for the athlete, and return what the lookup returned, their Strava
/// connection's status and whether the token still holds a seat.
async fn refresh_refused_with(
    resources: &Arc<ServerContext>,
    mock: &MockStrava,
    user_id: Uuid,
    tenant: TenantId,
    status: StatusCode,
    body: &str,
) -> (RefreshLookup, ConnectionStatus, bool) {
    *mock.refresh_rejection.lock().unwrap() = Some((status, body.to_owned()));
    let auth = AuthService::new(Arc::clone(resources) as Arc<dyn ToolRuntime>);
    let lookup = match auth
        .get_valid_token(user_id, "strava", Some(&tenant.to_string()))
        .await
    {
        Ok(None) => RefreshLookup::Refused,
        Err(OAuthError::RefreshUnavailable(provider)) if provider == "strava" => {
            RefreshLookup::Failed
        }
        Ok(Some(token)) => panic!("no usable token after HTTP {status}: {token:?}"),
        Err(e) => panic!("HTTP {status} is a transient refresh failure, not: {e}"),
    };
    let (connection, holds_seat) =
        connection_and_seat(&resources.common.repos, user_id, tenant).await;
    (lookup, connection, holds_seat)
}

/// The athlete's Strava connection status in `tenant`, and whether their token
/// holds a seat.
async fn connection_and_seat(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: TenantId,
) -> (ConnectionStatus, bool) {
    let status = repos
        .provider_connections
        .get_for_user(user_id, Some(tenant))
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.provider == "strava")
        .expect("the athlete has a Strava connection")
        .status;
    let holds_seat = repos
        .oauth_tokens
        .list_strava_seat_holders()
        .await
        .unwrap()
        .iter()
        .find(|h| h.user_id == user_id)
        .expect("the token is still listed")
        .counts_as_seat;
    (status, holds_seat)
}

/// A refresh Strava refuses for its rate limit (HTTP 429, a body naming the
/// application) or with a server error is transient: the lookup fails rather
/// than reporting no token, and the connection stays active and keeps its seat,
/// where reading the body alone recorded a rejected client and asked the
/// athlete to reconnect. A dead refresh token afterwards still flips it.
#[tokio::test]
#[serial]
async fn a_rate_limited_or_failing_refresh_leaves_the_connection_active() {
    let (base, mock) = mock_strava().await;
    let (resources, _service, _env) = service_pointed_at(&base).await;
    let (user_id, tenant) = athlete(&resources, "rate-limited").await;
    connect_expired(&resources.common.repos, user_id, tenant, None).await;

    for status in [
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::SERVICE_UNAVAILABLE,
    ] {
        assert_eq!(
            refresh_refused_with(
                &resources,
                &mock,
                user_id,
                tenant,
                status,
                STRAVA_RATE_LIMITED
            )
            .await,
            (RefreshLookup::Failed, ConnectionStatus::Active, true),
            "HTTP {status} is not a reason to reconnect"
        );
    }

    assert_eq!(
        refresh_refused_with(
            &resources,
            &mock,
            user_id,
            tenant,
            StatusCode::BAD_REQUEST,
            STRAVA_DEAD_REFRESH_TOKEN
        )
        .await,
        (RefreshLookup::Refused, ConnectionStatus::NeedsReauth, false),
        "a dead refresh token is a dead grant: reconnect, and the seat goes back"
    );
    assert_eq!(
        mock.token_requests.lock().unwrap().len(),
        3,
        "each call refreshed once"
    );
}

/// A refresh Strava refuses because it does not accept our client needs the
/// athlete to reconnect, but their grant is still authorized there, so the
/// token keeps its seat.
#[tokio::test]
#[serial]
async fn a_refresh_refused_over_our_client_keeps_the_seat() {
    let (base, mock) = mock_strava().await;
    let (resources, _service, _env) = service_pointed_at(&base).await;
    let (user_id, tenant) = athlete(&resources, "client-refused").await;
    connect_expired(&resources.common.repos, user_id, tenant, None).await;

    assert_eq!(
        refresh_refused_with(
            &resources,
            &mock,
            user_id,
            tenant,
            StatusCode::BAD_REQUEST,
            STRAVA_INVALID_CLIENT
        )
        .await,
        (RefreshLookup::Refused, ConnectionStatus::NeedsReauth, true)
    );
}

/// An expiry refresh of the athlete's pool-app token is in flight when a
/// reconnect onto the env app stores its token. The refreshed pair belongs to
/// the grant the reconnect replaced, so it is not written over the new token:
/// the row keeps the reconnect's tokens and app, and the caller gets the
/// reconnect's token.
#[tokio::test]
#[serial]
async fn a_refresh_in_flight_does_not_overwrite_the_token_a_reconnect_stored() {
    let (base, mock) = mock_strava().await;
    let (resources, _service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    repos
        .oauth_tokens
        .upsert_strava_pool_app("910017", POOL_SECRET, 5, Some("pool-q"))
        .await
        .unwrap();
    let (user_id, tenant) = athlete(&resources, "refresh-race").await;
    connect_expired(repos, user_id, tenant, Some("910017")).await;
    let reconnect = UserOAuthToken::new(
        user_id,
        tenant.to_string(),
        "strava".to_owned(),
        "access-of-the-reconnect".to_owned(),
        Some("refresh-of-the-reconnect".to_owned()),
        Some(Utc::now() + Duration::hours(6)),
        Some("read".to_owned()),
    );
    let reconnect_id = reconnect.id.clone();
    *mock.store_during_refresh.lock().unwrap() = Some(RacingStore {
        repos: Arc::clone(repos),
        token: reconnect,
    });

    let auth = AuthService::new(Arc::clone(&resources) as Arc<dyn ToolRuntime>);
    let token = auth
        .get_valid_token(user_id, "strava", Some(&tenant.to_string()))
        .await
        .expect("the lookup does not error")
        .expect("the reconnect's token is usable");

    assert_eq!(
        mock.token_requests.lock().unwrap().len(),
        1,
        "the refresh ran"
    );
    assert_eq!(token.access_token, "access-of-the-reconnect");
    assert_eq!(token.oauth_app_client_id, None);
    let stored = stored(repos, user_id, tenant).await;
    assert_eq!(stored.id, reconnect_id);
    assert_eq!(stored.access_token, "access-of-the-reconnect");
    assert_eq!(
        stored.refresh_token.as_deref(),
        Some("refresh-of-the-reconnect")
    );
    assert_eq!(stored.oauth_app_client_id, None, "the reconnect's app");
}

/// A refresh of a dead grant is in flight when the athlete reconnects: the
/// callback stores the new grant's token and re-arms the connection, and then
/// Strava refuses the old refresh token. The refusal is a verdict on the grant
/// the reconnect replaced, so the new connection stays active and keeps its
/// seat, and the caller gets the reconnect's token.
#[tokio::test]
#[serial]
async fn a_refresh_refused_after_a_reconnect_leaves_the_new_connection_active() {
    let (base, mock) = mock_strava().await;
    let (resources, _service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    let (user_id, tenant) = athlete(&resources, "refused-after-reconnect").await;
    connect_expired(repos, user_id, tenant, None).await;
    kill(repos, user_id, tenant).await;
    let reconnect = UserOAuthToken::new(
        user_id,
        tenant.to_string(),
        "strava".to_owned(),
        "access-of-the-reconnect".to_owned(),
        Some("refresh-of-the-reconnect".to_owned()),
        Some(Utc::now() + Duration::hours(6)),
        Some("read".to_owned()),
    );
    let reconnect_id = reconnect.id.clone();
    *mock.store_during_refresh.lock().unwrap() = Some(RacingStore {
        repos: Arc::clone(repos),
        token: reconnect,
    });
    *mock.refresh_rejection.lock().unwrap() = Some((
        StatusCode::BAD_REQUEST,
        STRAVA_DEAD_REFRESH_TOKEN.to_owned(),
    ));

    let auth = AuthService::new(Arc::clone(&resources) as Arc<dyn ToolRuntime>);
    let token = auth
        .get_valid_token(user_id, "strava", Some(&tenant.to_string()))
        .await
        .expect("a refused refresh is not an error");

    assert_eq!(
        mock.token_requests.lock().unwrap().len(),
        1,
        "the refresh ran"
    );
    assert_eq!(
        connection_and_seat(repos, user_id, tenant).await,
        (ConnectionStatus::Active, true),
        "the refusal of the replaced grant is no verdict on the reconnect"
    );
    let token = token.expect("the reconnect's token is usable");
    assert_eq!(token.access_token, "access-of-the-reconnect");
    assert_eq!(token.row_id, reconnect_id);
}

/// The nightly capture sweep refreshes an expired Strava token on its way to
/// the activity feed. Strava answering that refresh with its rate limit or a
/// server error says nothing about the athlete's grant, so the sweep records a
/// failed fetch and flags nothing: the connection stays active and keeps its
/// seat. The chat-triggered backfill decides on the same authentication, and
/// finds no auth-required tag in it to nudge a reconnect on.
#[tokio::test]
#[serial]
async fn the_capture_sweep_does_not_flag_a_rate_limited_or_failing_refresh() {
    let (base, mock) = mock_strava().await;
    let (resources, _service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let (user_id, tenant) = athlete(&resources, "swept-rate-limited").await;
    connect_expired(repos, user_id, tenant, None).await;

    for status in [
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::SERVICE_UNAVAILABLE,
    ] {
        *mock.refresh_rejection.lock().unwrap() = Some((status, STRAVA_RATE_LIMITED.to_owned()));

        let report = refresh_captures(&runtime, SweepBudget::default())
            .await
            .expect("refresh report");
        let line = report
            .connections
            .iter()
            .find(|c| c.user_id == user_id.to_string() && c.provider == "strava")
            .expect("the connection was walked");
        assert!(
            matches!(line.outcome, RefreshOutcome::Failed { .. }),
            "HTTP {status} is a failed fetch, not a lapsed session: {:?}",
            line.outcome
        );
        assert_eq!(report.flagged, 0, "HTTP {status} flags nothing");

        let refused = AuthService::new(Arc::clone(&runtime))
            .create_authenticated_provider("strava", user_id, Some(&tenant.to_string()))
            .await
            .err()
            .expect("no provider while the refresh fails");
        assert_eq!(
            auth_required_provider(&refused),
            None,
            "HTTP {status} asks for no reconnect: {:?}",
            refused.error
        );

        assert_eq!(
            connection_and_seat(repos, user_id, tenant).await,
            (ConnectionStatus::Active, true),
            "HTTP {status} is not a reason to reconnect"
        );
    }
    assert_eq!(
        mock.token_requests.lock().unwrap().len(),
        4,
        "each sweep and each authentication refreshed once"
    );
}

/// An athlete holds a live grant on a pool app in one tenant and a dead token
/// on that same app in another, and the app is being drained. Connecting in a
/// third tenant lands them on the env app, and the dead token's grant is not
/// revoked: Strava keeps one grant per athlete and app, so withdrawing it
/// would kill the live one too.
#[tokio::test]
#[serial]
async fn a_dead_token_on_an_app_with_a_live_grant_elsewhere_is_not_revoked() {
    let (base, mock) = mock_strava().await;
    let (resources, service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    repos
        .oauth_tokens
        .upsert_strava_pool_app("910018", POOL_SECRET, 5, Some("pool-r"))
        .await
        .unwrap();
    let (user_id, first, dead_in) = athlete_in_two_tenants(&resources, "live-and-dead").await;
    let (_, live_in) = athlete(&resources, "live-and-dead-third-owner").await;
    repos
        .users
        .update_tenant_id(user_id, live_in)
        .await
        .unwrap();
    connect(repos, user_id, live_in, Some("910018")).await;
    connect(repos, user_id, dead_in, Some("910018")).await;
    kill(repos, user_id, dead_in).await;
    repos
        .oauth_tokens
        .set_strava_pool_app_enabled("910018", false)
        .await
        .unwrap();

    let client_id = reconnect(&service, user_id, first).await;

    assert_eq!(client_id, ENV_CLIENT_ID, "a drained app takes no connect");
    assert!(
        mock.revocations.lock().unwrap().is_empty(),
        "the pool app's grant is live in another tenant"
    );
    assert_eq!(
        stored(repos, user_id, live_in).await.access_token,
        "access-of-the-replaced-grant",
        "the live grant's token is untouched"
    );
    assert_eq!(
        stored(repos, user_id, first).await.oauth_app_client_id,
        None
    );
}

/// A token in another tenant whose connection needs re-authorizing over our
/// own client credentials still holds a grant Strava counts, so a move to
/// another app leaves it alone rather than revoking it as a dead one.
#[tokio::test]
#[serial]
async fn a_needs_reauth_over_our_client_elsewhere_is_a_live_grant_left_alone() {
    let (base, mock) = mock_strava().await;
    let (resources, service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    repos
        .oauth_tokens
        .upsert_strava_pool_app("910019", POOL_SECRET, 5, Some("pool-s"))
        .await
        .unwrap();
    let (user_id, first, second) =
        athlete_in_two_tenants(&resources, "client-refused-elsewhere").await;
    connect(repos, user_id, second, Some("910019")).await;
    repos
        .provider_connections
        .mark_needs_reauth(
            user_id,
            second,
            "strava",
            Some("invalid_client"),
            Utc::now(),
        )
        .await
        .unwrap();
    repos
        .oauth_tokens
        .set_strava_pool_app_enabled("910019", false)
        .await
        .unwrap();

    let client_id = reconnect(&service, user_id, first).await;

    assert_eq!(client_id, ENV_CLIENT_ID);
    assert!(
        mock.revocations.lock().unwrap().is_empty(),
        "a grant over our own client credentials is still authorized at Strava"
    );
}

/// The provider a tool call gets for a pool-app token refreshes on its own
/// under that pool app's client, not the env app's, and writes the refreshed
/// pair back over the row it was read from.
#[tokio::test]
#[serial]
async fn a_provider_built_for_a_pool_token_refreshes_under_its_own_app() {
    let (base, mock) = mock_strava().await;
    let (resources, _service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    repos
        .oauth_tokens
        .upsert_strava_pool_app("910021", POOL_SECRET, 5, Some("pool-u"))
        .await
        .unwrap();
    let (user_id, tenant) = athlete(&resources, "provider-refresh").await;
    connect(repos, user_id, tenant, Some("910021")).await;
    // Valid for the lookup, which refreshes within five minutes of expiry,
    // and inside the provider's own five-minute window two seconds later.
    let nearly_expired = UserOAuthToken::new(
        user_id,
        tenant.to_string(),
        "strava".to_owned(),
        "access-of-the-replaced-grant".to_owned(),
        Some(REPLACED_REFRESH.to_owned()),
        Some(Utc::now() + Duration::minutes(5) + Duration::seconds(2)),
        Some("read".to_owned()),
    )
    .with_oauth_app_client_id(Some("910021".to_owned()));
    repos
        .oauth_tokens
        .upsert_token(&nearly_expired)
        .await
        .unwrap();

    let auth = AuthService::new(Arc::clone(&resources) as Arc<dyn ToolRuntime>);
    let provider = auth
        .create_authenticated_provider("strava", user_id, Some(&tenant.to_string()))
        .await
        .unwrap_or_else(|e| panic!("the provider is built: {:?}", e.error));
    assert!(
        mock.token_requests.lock().unwrap().is_empty(),
        "the lookup did not refresh the token"
    );
    sleep(StdDuration::from_millis(2500)).await;
    provider
        .refresh_token_if_needed()
        .await
        .expect("the provider refreshes against the mock");

    let refreshes = mock.token_requests.lock().unwrap().clone();
    assert_eq!(refreshes.len(), 1, "{refreshes:?}");
    assert!(
        refreshes[0].1.contains("client_id=910021")
            && refreshes[0]
                .1
                .contains(&format!("client_secret={POOL_SECRET}")),
        "the provider refreshes as the issuing pool app: {}",
        refreshes[0].1
    );
    let token = stored(repos, user_id, tenant).await;
    assert_eq!(token.id, nearly_expired.id);
    assert_eq!(token.access_token, "access-of-the-new-grant");
    assert_eq!(token.oauth_app_client_id.as_deref(), Some("910021"));
}

/// The mobile app's authorize route names the app the seat rules pick and
/// pins it on the state it stores, so the callback spends the code under that
/// app's client.
#[tokio::test]
#[serial]
async fn the_mobile_authorize_route_pins_the_pool_app_for_the_exchange() {
    let (base, mock) = mock_strava().await;
    let (resources, service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    repos
        .oauth_tokens
        .upsert_strava_pool_app("910022", POOL_SECRET, 5, Some("pool-v"))
        .await
        .unwrap();
    let (user_id, tenant) = athlete(&resources, "mobile-reconnect").await;
    connect(repos, user_id, tenant, None).await;
    kill(repos, user_id, tenant).await;
    fill_env_app(repos).await;
    let user = repos.users.get_global(user_id).await.unwrap().unwrap();
    let jwt = resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &user,
            &resources.auth.jwks_manager,
            Some(tenant.to_string()),
        )
        .unwrap();

    let response = AuthRoutes::routes(resources.auth_routes_context())
        .oneshot(
            Request::get("/api/oauth/mobile/init/strava")
                .header("authorization", format!("Bearer {jwt}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    let url = body["authorization_url"].as_str().unwrap();
    assert!(
        url.contains("client_id=910022"),
        "the URL names the pool app: {url}"
    );

    service
        .handle_callback("auth-code", body["state"].as_str().unwrap(), "strava")
        .await
        .expect("the exchange succeeds under the pinned app");

    let exchanges = mock.token_requests.lock().unwrap().clone();
    assert_eq!(exchanges.len(), 1, "{exchanges:?}");
    assert!(
        exchanges[0].1.contains("client_id=910022"),
        "the code is spent under the app the URL named: {}",
        exchanges[0].1
    );
    assert_eq!(
        stored(repos, user_id, tenant)
            .await
            .oauth_app_client_id
            .as_deref(),
        Some("910022")
    );
}

/// Run `sql` against the test database with text binds, on whichever backend
/// `DATABASE_URL` opened.
async fn run_sql(resources: &ServerContext, sql: &str, binds: &[&str]) {
    match &*resources.agent.database {
        Database::SQLite(db) => {
            let mut query = sqlx::query(sql);
            for bind in binds {
                query = query.bind(*bind);
            }
            query.execute(db.pool()).await.unwrap();
        }
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(db) => {
            let mut query = sqlx::query(sql);
            for bind in binds {
                query = query.bind(*bind);
            }
            query.execute(db.pool()).await.unwrap();
        }
    }
}

/// Two refreshes of one expired row race: the other one lands first and
/// spends the refresh token, and Strava refuses this one's reuse of it. The
/// refusal is a verdict on a refresh token already replaced, and the row keeps
/// its `id` across a refresh, so the guard reads it as it was read: the
/// connection stays active and keeps its seat, and the caller gets the pair the
/// other refresh stored.
#[tokio::test]
#[serial]
async fn a_refresh_refused_after_a_concurrent_refresh_landed_leaves_the_connection_active() {
    let (base, mock) = mock_strava().await;
    let (resources, _service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    let (user_id, tenant) = athlete(&resources, "concurrent-refresh").await;
    connect_expired(repos, user_id, tenant, None).await;
    let row_id = stored(repos, user_id, tenant).await.id;
    *mock.refresh_during_refresh.lock().unwrap() = Some((Arc::clone(repos), user_id, tenant));
    *mock.refresh_rejection.lock().unwrap() = Some((
        StatusCode::BAD_REQUEST,
        STRAVA_DEAD_REFRESH_TOKEN.to_owned(),
    ));

    let token = AuthService::new(Arc::clone(&resources) as Arc<dyn ToolRuntime>)
        .get_valid_token(user_id, "strava", Some(&tenant.to_string()))
        .await
        .expect("a refused refresh is not an error");

    assert_eq!(
        connection_and_seat(repos, user_id, tenant).await,
        (ConnectionStatus::Active, true),
        "the refusal of a spent refresh token is no verdict on the grant"
    );
    let token = token.expect("the concurrent refresh's token is usable");
    assert_eq!(token.access_token, CONCURRENT_ACCESS);
    assert_eq!(token.row_id, row_id, "a refresh keeps the row's id");
}

/// The athlete's elected Strava connection needs a refresh Strava answers with
/// its rate limit, and their WHOOP connection holds a ride of its own. The
/// window is served from WHOOP, and the answer says Strava could not be
/// reached, with its connection intact, rather than asking for a reconnect.
#[tokio::test]
#[serial]
async fn a_rate_limited_primary_serves_the_window_its_sibling_holds() {
    let (base, mock) = mock_strava().await;
    let (resources, _service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    let (user_id, tenant) = athlete(&resources, "rate-limited-primary").await;
    repos
        .provider_connections
        .register_connection(user_id, tenant, "whoop", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    let ride = ActivityBuilder::new(
        "whoop-ride-1".to_owned(),
        "Sortie".to_owned(),
        SportType::Ride,
        Utc::now() - Duration::days(2),
        5_400,
        "whoop".to_owned(),
    )
    .build();
    repos
        .activity_cache
        .upsert_activities(user_id, &tenant, "whoop", &[ride])
        .await
        .unwrap();
    // Strava second, so it is elected.
    connect_expired(repos, user_id, tenant, None).await;
    *mock.refresh_rejection.lock().unwrap() = Some((
        StatusCode::TOO_MANY_REQUESTS,
        STRAVA_RATE_LIMITED.to_owned(),
    ));

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant.to_string())
        .with_auth_method("jwt_bearer");
    let response = GetActivitiesTool
        .execute(&runtime, &ctx, json!({ "limit": 10, "mode": "summary" }))
        .await;

    let payload = response
        .structured_content
        .expect("tool result carries structured content");
    assert!(
        payload.get("error").is_none(),
        "a rate-limited primary must not blank a turn its sibling can answer: {payload}"
    );
    let ids: Vec<&str> = payload["activities"]
        .as_array()
        .expect("activities array present")
        .iter()
        .filter_map(|a| a.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(ids, vec!["whoop-ride-1"]);
    assert_eq!(payload["provider"].as_str(), Some("whoop"));
    assert!(
        payload.get("reconnect_required").is_none(),
        "a rate limit asks for no reconnect: {payload}"
    );
    let caveat = &payload["provider_unavailable"];
    assert_eq!(caveat["provider"].as_str(), Some("strava"), "{payload}");
    assert!(
        caveat["note"]
            .as_str()
            .is_some_and(|note| note.contains("do not ask the athlete to reconnect")),
        "{caveat}"
    );
    assert_eq!(
        connection_and_seat(repos, user_id, tenant).await,
        (ConnectionStatus::Active, true)
    );
}

/// The same rate-limited Strava primary, reached by the prefetch route rather
/// than the tool loop: a grounded turn's window is loaded before the model runs,
/// and only the prose list is injected. The note saying Strava's sessions are
/// missing has to come with it, or the partial window reads as the athlete's
/// whole training and the model plans on top of rides it cannot see.
#[cfg(feature = "client-chat")]
#[tokio::test]
#[serial]
async fn a_rate_limited_primary_is_named_in_the_prefetched_window() {
    let (base, mock) = mock_strava().await;
    let (resources, _service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    let (user_id, tenant) = athlete(&resources, "rate-limited-prefetch").await;
    repos
        .provider_connections
        .register_connection(user_id, tenant, "whoop", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    let ride = ActivityBuilder::new(
        "whoop-ride-2".to_owned(),
        "Sortie du matin".to_owned(),
        SportType::Ride,
        Utc::now() - Duration::days(2),
        5_400,
        "whoop".to_owned(),
    )
    .build();
    repos
        .activity_cache
        .upsert_activities(user_id, &tenant, "whoop", &[ride])
        .await
        .unwrap();
    // Strava second, so it is elected.
    connect_expired(repos, user_id, tenant, None).await;
    *mock.refresh_rejection.lock().unwrap() = Some((
        StatusCode::TOO_MANY_REQUESTS,
        STRAVA_RATE_LIMITED.to_owned(),
    ));

    let executor = Arc::new(
        UniversalExecutor::new(resources.clone() as Arc<dyn ToolRuntime>)
            .with_scopes(OAuthScope::self_grant()),
    );
    let window = prefetch_activity_context(
        &executor,
        &user_id.to_string(),
        tenant,
        &agentless_activity_window(),
    )
    .await
    .expect("the window is served from WHOOP");
    let mut messages = vec![
        ChatMessage::system("system prompt"),
        ChatMessage::user("comment s'est passée ma semaine?"),
    ];
    assert!(inject_activity_refresh(&mut messages, &window, ""));

    let injected = &messages[1].content;
    assert!(injected.contains("Sortie du matin"), "{injected}");
    let expected_note = provider_unavailable_note("strava");
    let note = expected_note["note"]
        .as_str()
        .expect("the caveat has a note");
    assert!(
        injected.contains(note),
        "the injected window must say Strava is missing from it: {injected}"
    );
}

/// A connection an earlier refusal flagged holds a grant only a reconnect
/// restores, and the athlete was already told so by the push and the app's
/// badge. A later refresh of that grant Strava answers with its rate limit or
/// a server error revives nothing: the lookup still reports no usable token,
/// so the chat asks for the same reconnect, rather than telling the model the
/// connection is intact and not to ask for one.
#[tokio::test]
#[serial]
async fn a_transient_refresh_of_a_refused_grant_still_asks_for_a_reconnect() {
    let (base, mock) = mock_strava().await;
    let (resources, _service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    let (user_id, tenant) = athlete(&resources, "refused-then-rate-limited").await;
    connect_expired(repos, user_id, tenant, None).await;
    kill(repos, user_id, tenant).await;

    for status in [
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::SERVICE_UNAVAILABLE,
    ] {
        assert_eq!(
            refresh_refused_with(
                &resources,
                &mock,
                user_id,
                tenant,
                status,
                STRAVA_RATE_LIMITED
            )
            .await,
            (RefreshLookup::Refused, ConnectionStatus::NeedsReauth, false),
            "HTTP {status} over a refused grant still needs the reconnect"
        );
        let refused = AuthService::new(Arc::clone(&resources) as Arc<dyn ToolRuntime>)
            .create_authenticated_provider("strava", user_id, Some(&tenant.to_string()))
            .await
            .err()
            .expect("no provider over a refused grant");
        assert_eq!(
            auth_required_provider(&refused).as_deref(),
            Some("strava"),
            "HTTP {status}: the turn becomes the reconnect link"
        );
        let text = refused.error.unwrap_or_default();
        assert!(!text.contains("intact"), "HTTP {status}: {text}");
    }
}

/// A refresh that fails without Strava refusing the grant reaches the caller
/// as text the model reads and the conversation stores. It names Strava and
/// says the connection is intact, and carries nothing Strava answered: not its
/// rate-limit body, and not the token pair of a success the client could not
/// parse, which the token client no longer copies into its error either.
#[tokio::test]
#[serial]
async fn a_transient_refresh_failure_carries_nothing_strava_answered() {
    let (base, mock) = mock_strava().await;
    let (resources, _service, _env) = service_pointed_at(&base).await;
    let (user_id, tenant) = athlete(&resources, "transient-text").await;
    connect_expired(&resources.common.repos, user_id, tenant, None).await;
    let unparsed_success = r#"{"access_token":"access-of-an-unparsed-success","refresh_token":"refresh-of-an-unparsed-success"}"#;

    for (status, body, answered) in [
        (
            StatusCode::TOO_MANY_REQUESTS,
            STRAVA_RATE_LIMITED,
            "Rate Limit Exceeded",
        ),
        (StatusCode::OK, unparsed_success, "unparsed-success"),
    ] {
        *mock.refresh_rejection.lock().unwrap() = Some((status, body.to_owned()));

        let refused = AuthService::new(Arc::clone(&resources) as Arc<dyn ToolRuntime>)
            .create_authenticated_provider("strava", user_id, Some(&tenant.to_string()))
            .await
            .err()
            .expect("no provider while the refresh fails");
        let text = refused.error.clone().expect("the failure is explained");
        assert!(
            !text.contains(answered) && !text.contains("errors"),
            "HTTP {status}: the model reads nothing Strava answered: {text}"
        );
        assert!(
            !text.contains("Authentication error"),
            "HTTP {status}: a standing grant is no authentication error: {text}"
        );
        assert!(
            text.contains("strava") && text.contains("intact"),
            "HTTP {status}: {text}"
        );
        assert_eq!(auth_required_provider(&refused), None);
    }

    let parse_error = refresh_strava_token(api_client(), ENV_CLIENT_ID, ENV_CLIENT_SECRET, "rt")
        .await
        .expect_err("an unparsable success is an error");
    assert!(
        !parse_error.to_string().contains("unparsed-success"),
        "a successful body is the token pair and stays out of the error: {parse_error}"
    );
}

/// A Strava token row that cannot be read (its ciphertext does not decrypt, as
/// every row reads after an encryption-key misconfiguration) is an error, not a
/// missing token: the lookup fails, and the capture sweep records a failed
/// fetch and flags nothing, where reading it as "no token" disconnected the
/// athlete and freed their seat.
#[tokio::test]
#[serial]
async fn an_unreadable_token_row_is_an_error_and_flags_nothing() {
    let (base, _mock) = mock_strava().await;
    let (resources, _service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let (user_id, tenant) = athlete(&resources, "unreadable-row").await;
    connect(repos, user_id, tenant, None).await;
    let row_id = stored(repos, user_id, tenant).await.id;
    run_sql(
        &resources,
        "UPDATE user_oauth_tokens SET access_token = $1 WHERE id = $2",
        &["not-a-ciphertext", &row_id],
    )
    .await;

    let lookup = AuthService::new(Arc::clone(&runtime))
        .get_valid_token(user_id, "strava", Some(&tenant.to_string()))
        .await;
    assert!(
        matches!(&lookup, Err(OAuthError::TokenStoreUnavailable(p)) if p == "strava"),
        "an unreadable row is a failed read: {lookup:?}"
    );

    // What the chat path hands the model, and the conversation stores: the
    // provider and a retry, not the store's text labelled an authentication
    // error, which reads as a reason to reconnect an intact connection.
    let expected = OAuthError::TokenStoreUnavailable("strava".to_owned()).to_string();
    let refused = AuthService::new(Arc::clone(&runtime))
        .create_authenticated_provider("strava", user_id, Some(&tenant.to_string()))
        .await
        .err()
        .expect("no provider over an unreadable token");
    assert_eq!(refused.error.as_deref(), Some(expected.as_str()));
    assert_eq!(auth_required_provider(&refused), None);
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant.to_string())
        .with_auth_method("jwt_bearer");
    let payload = GetActivitiesTool
        .execute(&runtime, &ctx, json!({ "limit": 10, "mode": "summary" }))
        .await
        .structured_content
        .expect("tool result carries structured content");
    assert_eq!(
        payload["error"].as_str(),
        Some(expected.as_str()),
        "{payload}"
    );

    let report = refresh_captures(&runtime, SweepBudget::default())
        .await
        .expect("refresh report");
    let line = report
        .connections
        .iter()
        .find(|c| c.user_id == user_id.to_string() && c.provider == "strava")
        .expect("the connection was walked");
    assert!(
        matches!(line.outcome, RefreshOutcome::Failed { .. }),
        "a failed read is a failed fetch: {:?}",
        line.outcome
    );
    assert_eq!(report.flagged, 0);
    assert_eq!(
        connection_and_seat(repos, user_id, tenant).await,
        (ConnectionStatus::Active, true)
    );
}

/// An expired token of a provider whose refresh endpoint this service does not
/// call has nothing a refresh here can renew: the lookup reports no usable
/// token, so the athlete is asked to reconnect, rather than an error saying a
/// later request retries a refresh that is never attempted.
#[tokio::test]
#[serial]
async fn an_expired_token_no_refresh_here_renews_asks_for_a_reconnect() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, tenant) = athlete(&resources, "no-refresh-endpoint").await;
    let token = UserOAuthToken::new(
        user_id,
        tenant.to_string(),
        "garmin".to_owned(),
        "access-of-an-expired-garmin-grant".to_owned(),
        Some("refresh-of-an-expired-garmin-grant".to_owned()),
        Some(Utc::now() - Duration::hours(1)),
        Some("read".to_owned()),
    );
    resources
        .common
        .repos
        .oauth_tokens
        .upsert_token(&token)
        .await
        .unwrap();

    let lookup = AuthService::new(Arc::clone(&resources) as Arc<dyn ToolRuntime>)
        .get_valid_token(user_id, "garmin", Some(&tenant.to_string()))
        .await;

    assert!(matches!(lookup, Ok(None)), "{lookup:?}");
}

/// An athlete holds a live grant on a pool app in one tenant and a dead token
/// on it in another, the app is being drained, and they reconnect in the
/// second tenant. When their tokens cannot be listed, whether the dead token's
/// app carries a live grant elsewhere is unknown, so it is not revoked:
/// Strava keeps one grant per athlete and app, and revoking it would
/// disconnect the live tenant too.
#[tokio::test]
#[serial]
async fn a_reconnect_that_cannot_list_the_athletes_tokens_revokes_nothing() {
    let (base, mock) = mock_strava().await;
    let (resources, service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    repos
        .oauth_tokens
        .upsert_strava_pool_app("910023", POOL_SECRET, 5, Some("pool-w"))
        .await
        .unwrap();
    let (user_id, live_in, dead_in) = athlete_in_two_tenants(&resources, "unlisted").await;
    connect(repos, user_id, live_in, Some("910023")).await;
    connect(repos, user_id, dead_in, Some("910023")).await;
    kill(repos, user_id, dead_in).await;
    repos
        .oauth_tokens
        .set_strava_pool_app_enabled("910023", false)
        .await
        .unwrap();
    let authorization = service
        .get_auth_url(user_id, dead_in, "strava")
        .await
        .expect("the env app has room");
    // The listing reads this table; nothing else the callback needs does.
    run_sql(&resources, "DROP TABLE user_oauth_app_credentials", &[]).await;

    service
        .handle_callback("auth-code", &authorization.state, "strava")
        .await
        .expect("the connect completes without the listing");

    assert!(
        mock.revocations.lock().unwrap().is_empty(),
        "an unknown live grant elsewhere is not revoked"
    );
    assert_eq!(
        stored(repos, user_id, dead_in).await.access_token,
        "access-of-the-new-grant",
        "the new token is stored"
    );
    assert_eq!(
        stored(repos, user_id, live_in).await.access_token,
        "access-of-the-replaced-grant",
        "the live grant's token is untouched"
    );
}

/// An athlete whose env-app grant is live in one tenant connects in another,
/// stays on the env app, and loses a race to a token another connect stored on
/// a pool app. The fresh grant nothing stores is the athlete's env-app grant,
/// the one the first tenant holds, so it is not revoked.
#[tokio::test]
#[serial]
async fn a_lost_race_leaves_a_fresh_grant_the_athlete_holds_elsewhere() {
    let (base, mock) = mock_strava().await;
    let (resources, service, _env) = service_pointed_at(&base).await;
    let repos = &resources.common.repos;
    let (user_id, live_in, racing_in) = athlete_in_two_tenants(&resources, "raced-live").await;
    connect(repos, user_id, live_in, None).await;
    let authorization = service
        .get_auth_url(user_id, racing_in, "strava")
        .await
        .expect("the env app has room");
    let winner = UserOAuthToken::new(
        user_id,
        racing_in.to_string(),
        "strava".to_owned(),
        "access-of-the-racing-grant".to_owned(),
        Some("refresh-of-the-racing-grant".to_owned()),
        Some(Utc::now() + Duration::hours(6)),
        Some("read".to_owned()),
    )
    .with_oauth_app_client_id(Some("910024".to_owned()));
    *mock.racing_store.lock().unwrap() = Some(RacingStore {
        repos: Arc::clone(repos),
        token: winner,
    });

    let error = service
        .handle_callback("auth-code", &authorization.state, "strava")
        .await
        .expect_err("the other connect's token landed first");

    assert_eq!(error.code, ErrorCode::ResourceLocked);
    assert!(
        mock.revocations.lock().unwrap().is_empty(),
        "revoking the fresh env-app grant would withdraw the live one"
    );
    assert_eq!(
        stored(repos, user_id, live_in).await.access_token,
        "access-of-the-replaced-grant"
    );
}

/// The health-sync scheduler reads credentials through the same refresh. A
/// refresh Strava answers with its rate limit is that provider's transient
/// failure, neither expired credentials nor a failed store read, and the
/// connection stays active.
#[cfg(feature = "health-sync")]
#[tokio::test]
#[serial]
async fn a_sync_credential_read_reports_a_rate_limited_refresh_as_the_providers() {
    let (base, mock) = mock_strava().await;
    let (resources, _service, _env) = service_pointed_at(&base).await;
    install_health_sync_refresher(&resources);
    let (user_id, tenant) = athlete(&resources, "sync-rate-limited").await;
    connect_expired(&resources.common.repos, user_id, tenant, None).await;
    *mock.refresh_rejection.lock().unwrap() = Some((
        StatusCode::TOO_MANY_REQUESTS,
        STRAVA_RATE_LIMITED.to_owned(),
    ));

    let storage = resources
        .fitness
        .sync_storage
        .as_ref()
        .expect("health sync is wired");
    let error = storage
        .get_credentials(&user_id.to_string(), "strava")
        .await
        .expect_err("no credentials while the refresh fails");

    assert!(
        matches!(&error, EnformeError::ProviderError { provider, .. } if provider == "strava"),
        "{error}"
    );
    assert_eq!(
        connection_and_seat(&resources.common.repos, user_id, tenant).await,
        (ConnectionStatus::Active, true)
    );
}
