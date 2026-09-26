// ABOUTME: carnet#550 — the connection status the SDK bridge polls turns connected once its provider flow completes
// ABOUTME: Drives the real start routes, the callback against a mocked Strava token endpoint, and get_connection_status
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The SDK bridge's `connect_provider` opens the provider's authorization
//! page and then asks Dravr, every few seconds, for the provider's
//! connection status through `get_connection_status`, the tool it already
//! reads to decide the provider is connected before it starts a flow. Dravr
//! posts nothing to the bridge's host, so a bridge on any machine learns the
//! outcome.
//!
//! These tests prove the status it polls moves from `disconnected` to
//! `connected` when a flow started on either route the bridge uses — the
//! initiate route in session mode, the mobile init mint in api-key mode —
//! completes, and that another athlete can neither start that flow nor see
//! it complete.

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
use pierre_core::models::TenantId;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_mcp_server::mcp::resources::ServerContext;
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
    let request = Request::get(uri)
        .header("authorization", format!("Bearer {jwt}"))
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

/// Start a flow on the initiate route the bridge fetches in session mode and
/// return the state its provider redirect carries.
async fn start_on_initiate(resources: &ServerContext, user_id: Uuid, jwt: &str) -> String {
    let uri = format!("/api/oauth/auth/strava/{user_id}");
    let (status, headers, body) = get_with_session(resources, &uri, jwt).await;
    assert_eq!(
        status,
        StatusCode::FOUND,
        "{uri} starts the flow: {}",
        String::from_utf8_lossy(&body)
    );
    Url::parse(headers[header::LOCATION].to_str().unwrap())
        .unwrap()
        .query_pairs()
        .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
        .expect("the provider redirect carries the flow's state")
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

/// The flow the bridge starts in session mode: the status reads disconnected
/// while the athlete is on the provider's page and connected once Dravr has
/// completed the callback, with no request to the bridge's host in between.
#[tokio::test]
#[serial]
async fn the_polled_status_turns_connected_when_the_bridge_flow_completes() {
    let strava = mock_strava().await;
    let (resources, service, executor, _env) = dravr(&strava).await;
    let (user_id, tenant_id, jwt) = athlete(&resources).await;

    let state = start_on_initiate(&resources, user_id, &jwt).await;

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

/// The flow the bridge mints in api-key mode on the mobile init route reads
/// the same way: disconnected until its callback lands, connected after.
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

/// Another athlete's bridge can neither start this athlete's flow nor see it
/// complete: the initiate route refuses a user id that is not the session's,
/// and the status it polls stays disconnected while the owner's turns
/// connected.
#[tokio::test]
#[serial]
async fn another_athletes_bridge_neither_starts_nor_sees_the_flow() {
    let strava = mock_strava().await;
    let (resources, service, executor, _env) = dravr(&strava).await;
    let (owner_id, owner_tenant, owner_jwt) = athlete(&resources).await;
    let (other_id, other_tenant, other_jwt) = athlete(&resources).await;

    let (refused, _, body) = get_with_session(
        &resources,
        &format!("/api/oauth/auth/strava/{owner_id}"),
        &other_jwt,
    )
    .await;
    assert_eq!(
        refused,
        StatusCode::FORBIDDEN,
        "{}",
        String::from_utf8_lossy(&body)
    );

    let state = start_on_initiate(&resources, owner_id, &owner_jwt).await;
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
