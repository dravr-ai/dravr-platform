// ABOUTME: The server side of the SDK bridge's connect_provider: the MCP tool that mints its flow and the status it polls
// ABOUTME: carnet#603 flow identity from the credential alone, a delegated grant included; carnet#550 status turns connected
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The SDK bridge's `connect_provider` has Dravr's `connect_provider` MCP
//! tool mint the provider's authorization page over the bridge's own `/mcp`
//! session, in every auth mode, opens it, and then asks Dravr, every few
//! seconds, for the provider's connection status through
//! `get_connection_status`, the tool it already reads to decide the provider
//! is connected before it starts a flow. Dravr posts nothing to the bridge's
//! host, so a bridge on any machine learns the outcome.
//!
//! An oauth-mode bridge holds a delegated grant, which every REST route
//! refuses and only MCP dispatch accepts, with its scopes enforced there:
//! `connect_provider` needs `profile:write`. The tool, like the launch route
//! the web app opens, takes the athlete from the credential that calls it and
//! from nothing else, so the retired initiate route that named a user id in
//! its path is gone, and no caller can start a flow for another account. The
//! status the bridge polls moves from `disconnected` to `connected` when a
//! flow completes, and another athlete's poll never sees it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::env;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use chrono::{Duration, Utc};
use pierre_auth::api_keys::{ApiKeyManager, ApiKeyTier, CreateApiKeyRequest};
use pierre_core::models::TenantId;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::mcp::McpRoutes;
use pierre_routes_auth::AuthRoutes;
use pierre_services::oauth_flow::OAuthService;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use serial_test::serial;
use tokio::net::TcpListener;
use tower::ServiceExt;
use url::Url;
use uuid::Uuid;

/// The access token the mocked Strava issues on every code exchange.
const EXCHANGED_ACCESS: &str = "access-from-the-exchange";

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

/// A Strava token endpoint that answers every code exchange with a fresh token.
async fn mock_strava() -> String {
    let app = Router::new().route(
        "/oauth/token",
        post(|| async {
            Json(json!({
                "access_token": EXCHANGED_ACCESS,
                "refresh_token": "refresh-from-the-exchange",
                "token_type": "Bearer",
                "expires_in": 21_600,
                "expires_at": (Utc::now() + Duration::hours(6)).timestamp(),
                "athlete": { "id": 4242 }
            }))
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    format!("http://{addr}")
}

/// The server a bridge talks to: its Strava code exchange reaches the mock,
/// the OAuth service completes callbacks, and the tool executor answers
/// `get_connection_status` as `/mcp` does. The registry reads
/// `PIERRE_STRAVA_TOKEN_URL` when the context is built, so the guard is set
/// first.
async fn dravr(
    strava: &str,
) -> (
    Arc<ServerContext>,
    OAuthService,
    UniversalToolExecutor,
    EnvGuard,
) {
    let guard = EnvGuard::set(&[
        ("PIERRE_STRAVA_TOKEN_URL", format!("{strava}/oauth/token")),
        ("STRAVA_CLIENT_ID", "poll-test-client".to_owned()),
        ("STRAVA_CLIENT_SECRET", "poll-test-secret".to_owned()),
    ]);
    let resources = common::create_test_server_resources().await.unwrap();
    let service = OAuthService::new(resources.data(), Arc::clone(&resources.common.config));
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let executor = UniversalToolExecutor::new(runtime).with_scopes(OAuthScope::self_grant());
    (resources, service, executor, guard)
}

/// An athlete with a tenant of their own, and a session token for them.
async fn athlete(resources: &ServerContext) -> (Uuid, TenantId, String) {
    let email = format!("poll-{}@example.com", Uuid::new_v4());
    let (user_id, _, tenant_id) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .unwrap();
    let user = resources
        .common
        .repos
        .users
        .get_global(user_id)
        .await
        .unwrap()
        .unwrap();
    let jwt = resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &user,
            &resources.auth.jwks_manager,
            Some(tenant_id.to_string()),
        )
        .unwrap();
    (user_id, tenant_id, jwt)
}

/// `GET uri` on the auth routes with the athlete's session.
async fn get_with_session(
    resources: &ServerContext,
    uri: &str,
    jwt: &str,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    get_with_authorization(resources, uri, &format!("Bearer {jwt}")).await
}

/// `GET uri` on the auth routes with `authorization` as the whole
/// `Authorization` value.
async fn get_with_authorization(
    resources: &ServerContext,
    uri: &str,
    authorization: &str,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let request = Request::get(uri)
        .header("authorization", authorization)
        .body(Body::empty())
        .unwrap();
    let response = AuthRoutes::routes(resources.auth_routes_context())
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    (status, headers, body)
}

/// The `state` a launch answer's provider redirect carries, once the answer
/// is checked to be that redirect.
fn state_of_redirect(uri: &str, status: StatusCode, headers: &HeaderMap, body: &[u8]) -> String {
    assert_eq!(
        status,
        StatusCode::FOUND,
        "{uri} starts the flow: {}",
        String::from_utf8_lossy(body)
    );
    Url::parse(headers[header::LOCATION].to_str().unwrap())
        .unwrap()
        .query_pairs()
        .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
        .expect("the provider redirect carries the flow's state")
}

/// Start a flow on the launch route the web app opens with a session and
/// return the state its provider redirect carries.
async fn start_on_launch(resources: &ServerContext, jwt: &str) -> String {
    let uri = "/api/oauth/authorize/strava";
    let (status, headers, body) = get_with_session(resources, uri, jwt).await;
    state_of_redirect(uri, status, &headers, &body)
}

/// The athlete a flow's state was minted for: its first `:` segment.
fn athlete_of_state(state: &str) -> Uuid {
    Uuid::parse_str(state.split(':').next().unwrap()).unwrap()
}

/// A stored API key for `user_id`, as the whole `Authorization` value a REST
/// route reads it from.
async fn api_key_for(resources: &ServerContext, user_id: Uuid) -> String {
    let (key, full_key) = ApiKeyManager::new()
        .create_api_key(
            user_id,
            CreateApiKeyRequest {
                name: "bridge-launch".to_owned(),
                description: None,
                tier: ApiKeyTier::Starter,
                rate_limit_requests: Some(1000),
                expires_in_days: None,
            },
        )
        .unwrap();
    resources.common.repos.api_keys.create(&key).await.unwrap();
    full_key
}

/// The grant an oauth-mode SDK bridge asks for: every delegable scope, which
/// is never the self grant, so REST refuses it and `/mcp` enforces it.
fn bridge_delegation() -> Vec<OAuthScope> {
    OAuthScope::ALL
        .into_iter()
        .filter(|scope| scope.is_delegable())
        .collect()
}

/// The `Authorization` value of a delegated OAuth grant for `user_id`, minted
/// the way the authorization server's token endpoint mints it for an
/// application: no active tenant, and the scopes the athlete consented to.
fn delegated_bearer(resources: &ServerContext, user_id: Uuid, grant: &[OAuthScope]) -> String {
    let scopes: Vec<String> = grant
        .iter()
        .map(|scope| scope.as_str().to_owned())
        .collect();
    let token = resources
        .auth
        .auth_manager
        .generate_oauth_access_token(&resources.auth.jwks_manager, &user_id, &scopes, &[], None)
        .unwrap();
    format!("Bearer {token}")
}

/// `connect_provider` for Strava over `/mcp`, as the bridge calls it: the
/// provider alone, the athlete taken from `authorization`.
async fn connect_over_mcp(
    resources: &Arc<ServerContext>,
    authorization: &str,
) -> (StatusCode, HeaderMap, Value) {
    let request = Request::post("/mcp")
        .header("authorization", authorization)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": "connect_provider", "arguments": { "provider": "strava" } }
            })
            .to_string(),
        ))
        .unwrap();
    let response = McpRoutes::routes(Arc::clone(resources))
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json = serde_json::from_slice(&body).unwrap_or(Value::Null);
    (status, headers, json)
}

/// What one poll of the bridge reads: `get_connection_status` for Strava,
/// asked as the athlete in their tenant.
async fn strava_status(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Value {
    let response = executor
        .execute_tool(UniversalRequest {
            user_id: user_id.to_string(),
            tool_name: "get_connection_status".to_owned(),
            parameters: json!({ "provider": "strava" }),
            protocol: "mcp".to_owned(),
            tenant_id: Some(tenant_id.to_string()),
            progress_token: None,
            cancellation_token: None,
            progress_reporter: None,
        })
        .await
        .unwrap();
    assert!(response.success, "{:?}", response.error);
    response.result.expect("the status tool answers a payload")
}

/// A flow started on the launch route: the status the bridge polls reads
/// disconnected while the athlete is on the provider's page and connected
/// once Dravr has completed the callback, with no request to the bridge's host
/// in between.
#[tokio::test]
#[serial]
async fn the_polled_status_turns_connected_when_the_bridge_flow_completes() {
    let strava = mock_strava().await;
    let (resources, service, executor, _env) = dravr(&strava).await;
    let (user_id, tenant_id, jwt) = athlete(&resources).await;

    let state = start_on_launch(&resources, &jwt).await;

    let pending = strava_status(&executor, user_id, tenant_id).await;
    assert_eq!(pending["provider"], "strava");
    assert_eq!(pending["status"], "disconnected");
    assert_eq!(pending["connected"], false);

    service
        .handle_callback("auth-code", &state, "strava")
        .await
        .expect("the exchange succeeds against the mock token endpoint");

    let connected = strava_status(&executor, user_id, tenant_id).await;
    assert_eq!(connected["status"], "connected");
    assert_eq!(connected["connected"], true);
    assert_eq!(connected["needs_reauth"], false);
    assert_eq!(connected["backend"], "oauth");
}

/// A flow minted on the native app's mobile init route reads the same way:
/// disconnected until its callback lands, connected after.
#[tokio::test]
#[serial]
async fn the_polled_status_turns_connected_when_a_minted_flow_completes() {
    let strava = mock_strava().await;
    let (resources, service, executor, _env) = dravr(&strava).await;
    let (user_id, tenant_id, jwt) = athlete(&resources).await;

    let (status, _, body) =
        get_with_session(&resources, "/api/oauth/mobile/init/strava", &jwt).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let minted: Value = serde_json::from_slice(&body).unwrap();
    let state = minted["state"].as_str().unwrap().to_owned();

    assert_eq!(
        strava_status(&executor, user_id, tenant_id).await["status"],
        "disconnected"
    );

    service
        .handle_callback("auth-code", &state, "strava")
        .await
        .expect("the minted flow completes");

    let connected = strava_status(&executor, user_id, tenant_id).await;
    assert_eq!(connected["status"], "connected");
    assert_eq!(connected["connected"], true);
}

/// Another athlete's bridge never sees this athlete's flow complete: the
/// status it polls stays disconnected while the owner's turns connected.
#[tokio::test]
#[serial]
async fn another_athletes_bridge_never_sees_the_flow_complete() {
    let strava = mock_strava().await;
    let (resources, service, executor, _env) = dravr(&strava).await;
    let (owner_id, owner_tenant, owner_jwt) = athlete(&resources).await;
    let (other_id, other_tenant, _) = athlete(&resources).await;

    let state = start_on_launch(&resources, &owner_jwt).await;
    service
        .handle_callback("auth-code", &state, "strava")
        .await
        .expect("the owner's flow completes");

    let owner = strava_status(&executor, owner_id, owner_tenant).await;
    assert_eq!(owner["status"], "connected");

    let other = strava_status(&executor, other_id, other_tenant).await;
    assert_eq!(other["status"], "disconnected");
    assert_eq!(other["connected"], false);
    let stored = resources
        .common
        .repos
        .oauth_tokens
        .get_token(other_id, other_tenant, "strava")
        .await
        .unwrap();
    assert!(
        stored.is_none(),
        "the owner's provider token never lands on another athlete"
    );
}

/// The route the bridge started from before carnet#603, which named its
/// athlete in the path, is gone: a valid session reaches a 404 there.
#[tokio::test]
#[serial]
async fn the_retired_initiate_route_answers_404() {
    let strava = mock_strava().await;
    let (resources, _, _, _env) = dravr(&strava).await;
    let (user_id, _, jwt) = athlete(&resources).await;

    let (status, headers, _) = get_with_session(
        &resources,
        &format!("/api/oauth/auth/strava/{user_id}"),
        &jwt,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(headers.get(header::LOCATION).is_none());
}

/// The launch route accepts both of the athlete's own credentials — a session
/// bearer, and an API key as the whole `Authorization` value — and mints the
/// flow for the athlete each one names.
#[tokio::test]
#[serial]
async fn the_launch_route_starts_the_flow_of_the_athlete_the_credential_names() {
    let strava = mock_strava().await;
    let (resources, _, _, _env) = dravr(&strava).await;
    let (session_athlete, _, jwt) = athlete(&resources).await;
    let (key_athlete, _, _) = athlete(&resources).await;
    let uri = "/api/oauth/authorize/strava";

    let (status, headers, body) = get_with_session(&resources, uri, &jwt).await;
    let session_state = state_of_redirect(uri, status, &headers, &body);
    assert_eq!(athlete_of_state(&session_state), session_athlete);

    let key = api_key_for(&resources, key_athlete).await;
    let (status, headers, body) = get_with_authorization(&resources, uri, &key).await;
    let key_state = state_of_redirect(uri, status, &headers, &body);
    assert_eq!(athlete_of_state(&key_state), key_athlete);
}

/// Identity comes from the credential and from nothing a caller writes: an
/// athlete who names another's user id — in the retired path or in a query
/// parameter — starts a flow for their own account or none, and completing it
/// connects the provider to them alone.
#[tokio::test]
#[serial]
async fn the_launch_route_refuses_another_athletes_identity() {
    let strava = mock_strava().await;
    let (resources, service, executor, _env) = dravr(&strava).await;
    let (owner_id, owner_tenant, _) = athlete(&resources).await;
    let (caller_id, caller_tenant, caller_jwt) = athlete(&resources).await;

    let (status, _, _) = get_with_session(
        &resources,
        &format!("/api/oauth/auth/strava/{owner_id}"),
        &caller_jwt,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let uri = format!("/api/oauth/authorize/strava?user_id={owner_id}");
    let (status, headers, body) = get_with_session(&resources, &uri, &caller_jwt).await;
    let state = state_of_redirect(&uri, status, &headers, &body);
    assert_eq!(athlete_of_state(&state), caller_id);

    service
        .handle_callback("auth-code", &state, "strava")
        .await
        .expect("the caller's own flow completes");

    assert_eq!(
        strava_status(&executor, caller_id, caller_tenant).await["status"],
        "connected"
    );
    assert_eq!(
        strava_status(&executor, owner_id, owner_tenant).await["status"],
        "disconnected"
    );
    let owner_token = resources
        .common
        .repos
        .oauth_tokens
        .get_token(owner_id, owner_tenant, "strava")
        .await
        .unwrap();
    assert!(
        owner_token.is_none(),
        "nothing lands on the athlete the caller named"
    );
}

/// An oauth-mode bridge's delegated grant starts its flow through the
/// `connect_provider` tool: the URL the tool returns carries a flow bound to
/// the athlete the grant names, completing it connects that athlete, and
/// another athlete's poll never sees it.
#[tokio::test]
#[serial]
async fn the_connect_provider_tool_mints_a_delegated_bridges_flow_for_its_athlete_only() {
    let strava = mock_strava().await;
    let (resources, service, executor, _env) = dravr(&strava).await;
    let (owner_id, owner_tenant, _) = athlete(&resources).await;
    let (other_id, other_tenant, _) = athlete(&resources).await;

    let (status, _, body) = connect_over_mcp(
        &resources,
        &delegated_bearer(&resources, owner_id, &bridge_delegation()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["isError"], json!(false), "{body}");
    let minted = &body["result"]["structuredContent"];
    assert_eq!(minted["provider"], "strava");
    assert_eq!(minted["status"], "pending_authorization");

    let url = Url::parse(minted["authorization_url"].as_str().unwrap()).unwrap();
    let query = |name: &str| {
        url.query_pairs()
            .find_map(|(key, value)| (key == name).then(|| value.into_owned()))
    };
    assert_eq!(query("client_id").as_deref(), Some("poll-test-client"));
    let state = query("state").expect("the minted URL carries the flow's state");
    assert_eq!(state, minted["state"].as_str().unwrap());
    assert_eq!(
        athlete_of_state(&state),
        owner_id,
        "the flow belongs to the athlete the grant names"
    );

    assert_eq!(
        strava_status(&executor, owner_id, owner_tenant).await["status"],
        "disconnected"
    );
    service
        .handle_callback("auth-code", &state, "strava")
        .await
        .expect("the tool's flow completes on the callback");

    let owner = strava_status(&executor, owner_id, owner_tenant).await;
    assert_eq!(owner["status"], "connected");
    assert_eq!(owner["connected"], true);
    let other = strava_status(&executor, other_id, other_tenant).await;
    assert_eq!(other["status"], "disconnected");
    let stored = resources
        .common
        .repos
        .oauth_tokens
        .get_token(other_id, other_tenant, "strava")
        .await
        .unwrap();
    assert!(
        stored.is_none(),
        "the grant's flow never lands on another athlete"
    );
}

/// The tool is served to a delegated grant only within its scopes, as MCP
/// dispatch serves every tool: linking a provider needs `profile:write`, and a
/// grant without it is refused with the RFC 6750 challenge naming it. The REST
/// launch routes stay the athlete's own: they refuse even the bridge's widest
/// grant.
#[tokio::test]
#[serial]
async fn a_delegated_grant_reaches_connect_provider_only_within_its_scopes() {
    let strava = mock_strava().await;
    let (resources, _, _, _env) = dravr(&strava).await;
    let (user_id, _, jwt) = athlete(&resources).await;

    let narrow = delegated_bearer(
        &resources,
        user_id,
        &[
            OAuthScope::FitnessRead,
            OAuthScope::FitnessWrite,
            OAuthScope::ProfileRead,
        ],
    );
    let (status, headers, body) = connect_over_mcp(&resources, &narrow).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let challenge = headers[header::WWW_AUTHENTICATE].to_str().unwrap();
    assert!(
        challenge.contains("error=\"insufficient_scope\"")
            && challenge.contains("scope=\"profile:write\""),
        "the challenge names the grant the tool needs: {challenge}"
    );

    let widest = delegated_bearer(&resources, user_id, &bridge_delegation());
    for uri in [
        "/api/oauth/authorize/strava",
        "/api/oauth/mobile/init/strava",
    ] {
        let (status, headers, body) = get_with_authorization(&resources, uri, &widest).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{uri} refuses a delegated grant: {}",
            String::from_utf8_lossy(&body)
        );
        assert!(headers.get(header::LOCATION).is_none());
    }

    // The athlete's own session still starts a flow on the launch route.
    let uri = "/api/oauth/authorize/strava";
    let (status, headers, body) = get_with_session(&resources, uri, &jwt).await;
    assert_eq!(
        athlete_of_state(&state_of_redirect(uri, status, &headers, &body)),
        user_id
    );
}
