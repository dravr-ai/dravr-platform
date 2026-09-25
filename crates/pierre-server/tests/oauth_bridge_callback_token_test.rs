// ABOUTME: carnet#550 — a provider OAuth flow the SDK bridge starts notifies that bridge, presenting the flow's own token
// ABOUTME: Drives the real start routes and callback against a mocked Strava token endpoint and a recording bridge listener
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The SDK bridge's local listener stores whatever provider tokens it accepts
//! and is reachable by every process on the machine, so it refuses a
//! provider-token POST that does not present the per-flow token it generated.
//! The bridge hands that token over in `X-Callback-Token` when it starts the
//! flow; these tests prove the server keeps it with the flow's state and
//! presents it — and only it — in the same header on the one notification
//! that flow sends, that a flow no bridge started notifies nobody, and that a
//! malformed token is refused before any flow starts. The listener's own refusal of an absent or
//! mismatched token is the SDK's, proven in `sdk/test/unit`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::env;
use std::sync::{Arc, Mutex};

use axum::body::{to_bytes, Body};
use axum::extract::Path;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use chrono::{Duration, Utc};
use pierre_core::models::TenantId;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_auth::AuthRoutes;
use pierre_services::oauth_flow::{AuthUrlOptions, OAuthService};
use serde_json::{json, Value};
use serial_test::serial;
use tokio::net::TcpListener;
use tower::ServiceExt;
use url::Url;
use uuid::Uuid;

/// The per-flow token one bridge listener generated (64 hex characters, as
/// the SDK mints it).
const BRIDGE_TOKEN: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

/// A second bridge flow's token, so each notification is shown to carry its
/// own flow's value rather than one the server holds globally.
const OTHER_BRIDGE_TOKEN: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";

/// The header a bridge presents its per-flow token in, both ways.
const CALLBACK_TOKEN_HEADER: &str = "x-callback-token";

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

/// One POST the bridge listener received.
#[derive(Debug, Clone)]
struct Notification {
    provider: String,
    callback_token: Option<String>,
    body: Value,
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

/// A bridge listener that records every provider-token POST and the
/// per-flow token it presented, and returns the port it holds.
async fn recording_bridge() -> (u16, Arc<Mutex<Vec<Notification>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);
    let app = Router::new().route(
        "/oauth/provider-callback/{provider}",
        post(
            move |Path(provider): Path<String>, headers: HeaderMap, Json(body): Json<Value>| {
                let recorder = Arc::clone(&recorder);
                async move {
                    let callback_token = headers
                        .get(CALLBACK_TOKEN_HEADER)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_owned);
                    recorder.lock().unwrap().push(Notification {
                        provider,
                        callback_token,
                        body,
                    });
                    Json(json!({ "success": true }))
                }
            },
        ),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (port, seen)
}

/// A server context whose Strava code exchange reaches the mock, and the OAuth
/// service that completes callbacks with the bridge listener on `bridge_port`.
/// The registry reads `PIERRE_STRAVA_TOKEN_URL` when the context is built, so
/// the guard is set first.
async fn service_with_bridge(
    strava: &str,
    bridge_port: u16,
) -> (Arc<ServerContext>, OAuthService, EnvGuard) {
    let guard = EnvGuard::set(&[
        ("PIERRE_STRAVA_TOKEN_URL", format!("{strava}/oauth/token")),
        ("STRAVA_CLIENT_ID", "bridge-test-client".to_owned()),
        ("STRAVA_CLIENT_SECRET", "bridge-test-secret".to_owned()),
    ]);
    let resources = common::create_test_server_resources().await.unwrap();
    let mut config = (*resources.common.config).clone();
    config.oauth_callback_port = bridge_port;
    let service = OAuthService::new(resources.data(), Arc::new(config));
    (resources, service, guard)
}

/// An athlete with a tenant of their own, and a session token for them.
async fn athlete(resources: &ServerContext) -> (Uuid, TenantId, String) {
    let email = format!("bridge-{}@example.com", Uuid::new_v4());
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

/// `GET uri` on the auth routes with the athlete's session, presenting a
/// bridge listener's per-flow token when one is given.
async fn get_with_session(
    resources: &ServerContext,
    uri: &str,
    jwt: &str,
    callback_token: Option<&str>,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut request = Request::get(uri).header("authorization", format!("Bearer {jwt}"));
    if let Some(token) = callback_token {
        request = request.header(CALLBACK_TOKEN_HEADER, token);
    }
    let response = AuthRoutes::routes(resources.auth_routes_context())
        .oneshot(request.body(Body::empty()).unwrap())
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

/// The `state` a provider authorization URL carries.
fn state_of(authorization_url: &str) -> String {
    Url::parse(authorization_url)
        .unwrap()
        .query_pairs()
        .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
        .expect("the authorization URL carries a state")
}

/// Start a flow on a redirecting start route and return the state its
/// provider redirect carries.
async fn start_by_redirect(
    resources: &ServerContext,
    uri: &str,
    jwt: &str,
    callback_token: Option<&str>,
) -> String {
    let (status, headers, body) = get_with_session(resources, uri, jwt, callback_token).await;
    assert_eq!(
        status,
        StatusCode::FOUND,
        "{uri} starts the flow: {}",
        String::from_utf8_lossy(&body)
    );
    state_of(headers[header::LOCATION].to_str().unwrap())
}

/// The bridge started the flow on the initiate route the SDK opens: the
/// callback's notification reaches it with that flow's token and the fresh
/// provider token, so the listener accepts it.
#[tokio::test]
#[serial]
async fn a_flow_the_bridge_starts_notifies_it_with_its_own_token() {
    let strava = mock_strava().await;
    let (bridge_port, seen) = recording_bridge().await;
    let (resources, service, _env) = service_with_bridge(&strava, bridge_port).await;
    let (user_id, _, jwt) = athlete(&resources).await;

    let state = start_by_redirect(
        &resources,
        &format!("/api/oauth/auth/strava/{user_id}"),
        &jwt,
        Some(BRIDGE_TOKEN),
    )
    .await;
    service
        .handle_callback("auth-code", &state, "strava")
        .await
        .expect("the exchange succeeds against the mock token endpoint");

    let notifications = seen.lock().unwrap().clone();
    assert_eq!(notifications.len(), 1, "{notifications:?}");
    assert_eq!(notifications[0].provider, "strava");
    assert_eq!(
        notifications[0].callback_token.as_deref(),
        Some(BRIDGE_TOKEN)
    );
    assert_eq!(notifications[0].body["access_token"], EXCHANGED_ACCESS);
    assert_eq!(
        notifications[0].body["refresh_token"],
        "refresh-from-the-exchange"
    );
}

/// Each flow presents its own token: one started on the authorize door and
/// one minted on the mobile init route (the SDK's api-key path) with another
/// bridge token each notify with the token they were started with.
#[tokio::test]
#[serial]
async fn every_flow_presents_the_token_it_was_started_with() {
    let strava = mock_strava().await;
    let (bridge_port, seen) = recording_bridge().await;
    let (resources, service, _env) = service_with_bridge(&strava, bridge_port).await;
    let (_, _, jwt) = athlete(&resources).await;

    let authorize_state = start_by_redirect(
        &resources,
        "/api/oauth/authorize/strava",
        &jwt,
        Some(BRIDGE_TOKEN),
    )
    .await;

    let (status, _, body) = get_with_session(
        &resources,
        "/api/oauth/mobile/init/strava",
        &jwt,
        Some(OTHER_BRIDGE_TOKEN),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let minted: Value = serde_json::from_slice(&body).unwrap();
    let mobile_state = minted["state"].as_str().unwrap().to_owned();

    service
        .handle_callback("auth-code", &mobile_state, "strava")
        .await
        .expect("the mobile-minted flow completes");
    service
        .handle_callback("auth-code", &authorize_state, "strava")
        .await
        .expect("the authorize-door flow completes");

    let presented: Vec<Option<String>> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|notification| notification.callback_token.clone())
        .collect();
    assert_eq!(
        presented,
        vec![
            Some(OTHER_BRIDGE_TOKEN.to_owned()),
            Some(BRIDGE_TOKEN.to_owned())
        ]
    );
}

/// A flow no bridge started — the web app's, the mobile app's — carries no
/// listener token, so its callback posts the provider token to nobody.
#[tokio::test]
#[serial]
async fn a_flow_no_bridge_started_notifies_no_bridge() {
    let strava = mock_strava().await;
    let (bridge_port, seen) = recording_bridge().await;
    let (resources, service, _env) = service_with_bridge(&strava, bridge_port).await;
    let (user_id, tenant_id, jwt) = athlete(&resources).await;

    let web_state = start_by_redirect(&resources, "/api/oauth/authorize/strava", &jwt, None).await;
    service
        .handle_callback("auth-code", &web_state, "strava")
        .await
        .expect("the web flow completes");

    let plain = service
        .get_auth_url(user_id, tenant_id, "strava", AuthUrlOptions::default())
        .await
        .unwrap();
    service
        .handle_callback("auth-code", &plain.state, "strava")
        .await
        .expect("the plain flow completes");

    assert!(
        seen.lock().unwrap().is_empty(),
        "no bridge started either flow, so none is sent the provider token"
    );
    let stored = resources
        .common
        .repos
        .oauth_tokens
        .get_token(user_id, tenant_id, "strava")
        .await
        .unwrap()
        .expect("the connection itself still lands");
    assert_eq!(stored.access_token, EXCHANGED_ACCESS);
}

/// A malformed per-flow token is refused on every start route before any
/// flow starts, so nothing unfit to send back as a header is ever stored.
#[tokio::test]
#[serial]
async fn a_malformed_callback_token_is_refused_before_any_flow_starts() {
    let strava = mock_strava().await;
    let (bridge_port, _seen) = recording_bridge().await;
    let (resources, _service, _env) = service_with_bridge(&strava, bridge_port).await;
    let (user_id, _, jwt) = athlete(&resources).await;

    let too_long = "a".repeat(129);
    let not_url_safe = format!("{}:{}", "a".repeat(20), "b".repeat(20));
    for (uri, token) in [
        (format!("/api/oauth/auth/strava/{user_id}"), "short"),
        ("/api/oauth/authorize/strava".to_owned(), too_long.as_str()),
        (
            "/api/oauth/mobile/init/strava".to_owned(),
            not_url_safe.as_str(),
        ),
    ] {
        let (status, _, body) = get_with_session(&resources, &uri, &jwt, Some(token)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        let refusal: Value = serde_json::from_slice(&body).unwrap();
        assert!(
            refusal
                .to_string()
                .contains("X-Callback-Token must be 32 to 128 characters"),
            "{uri}: {refusal}"
        );
    }
}
