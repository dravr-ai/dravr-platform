// ABOUTME: HTTP integration tests for POST /api/providers/intervals_icu/link-credentials
// ABOUTME: Pins the status codes — a rejected API key is 400, only a dead session is 401
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "provider-intervals-icu")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! End-to-end tests for the Intervals.icu link endpoint.
//!
//! The status code this endpoint returns for a *rejected API key* is load-bearing
//! well beyond this route. The shared `@pierre/api-client` response interceptor
//! treats any 401 as a dead session: it clears stored auth and fires the
//! sign-out callback. So when this handler answered a bad athlete-id/API-key
//! pair with 401, a mistyped key logged the athlete out of Dravr entirely.
//!
//! Intervals.icu is stubbed on loopback (the registry's default config carries
//! the base URL), so these tests never touch the network.

mod common;
mod helpers;

use common::{create_test_server_resources, create_test_user};
use helpers::axum_test::AxumTestRequest;

use axum::http::StatusCode;
use axum::Router;
use pierre_core::constants::oauth::INTERVALS_ICU;
use pierre_providers::intervals_icu_provider::default_config;
use pierre_providers::ProviderRegistry;
use serde_json::json;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// One-shot loopback stub standing in for Intervals.icu. Answers a single
/// request with `status_line` + `body` and returns the captured request head.
async fn stub_once(status_line: &'static str, body: &'static str) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
    let addr = listener.local_addr().expect("stub addr");
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut head = Vec::new();
        let mut buf = [0_u8; 1024];
        loop {
            let n = socket.read(&mut buf).await.expect("read request");
            if n == 0 {
                break;
            }
            head.extend_from_slice(&buf[..n]);
            if head.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let response = format!(
            "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write response");
        socket.flush().await.expect("flush response");
        String::from_utf8_lossy(&head).into_owned()
    });
    (format!("http://{addr}"), handle)
}

/// Stand up the auth router with the Intervals.icu provider pointed at `base_url`.
async fn setup(base_url: String) -> (Router, String) {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.coach.database).await.unwrap();
    // The handler resolves the active tenant from the token; a tenant-less token
    // fails earlier, on "No active tenant", and would never reach the provider.
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("list tenants")
        .first()
        .expect("user owns a tenant")
        .id;
    let token = resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &user,
            &resources.auth.jwks_manager,
            Some(tenant_id.to_string()),
        )
        .unwrap();

    let mut config = default_config();
    config.api_base_url = base_url;
    let mut registry = ProviderRegistry::new();
    registry.set_default_config(INTERVALS_ICU, config);

    let mut context = resources.auth_routes_context();
    context.provider_registry = Arc::new(registry);

    let router = pierre_routes_auth::AuthRoutes::routes(context);
    (router, format!("Bearer {token}"))
}

#[tokio::test]
async fn rejected_api_key_is_400_so_the_client_does_not_sign_the_user_out() {
    // Intervals.icu rejects the key. The athlete's Dravr session is untouched,
    // so the answer must be 400 (bad field in the request body) — never 401,
    // which the api-client interceptor reads as "session dead" and signs out on.
    let (base_url, stub) = stub_once("HTTP/1.1 401 Unauthorized", "{}").await;
    let (router, auth) = setup(base_url).await;

    let response = AxumTestRequest::post("/api/providers/intervals_icu/link-credentials")
        .header("authorization", &auth)
        .json(&json!({ "athlete_id": "i123456", "api_key": "wrong-key" }))
        .send(router)
        .await;

    assert_eq!(
        response.status_code(),
        StatusCode::BAD_REQUEST,
        "a rejected provider key is a bad request body, not a dead session"
    );
    assert_ne!(
        response.status_code(),
        StatusCode::UNAUTHORIZED,
        "401 here would clear the athlete's stored auth and sign them out"
    );

    let body: serde_json::Value = response.json();
    let rendered = body.to_string();
    assert!(
        rendered.contains("Intervals.icu rejected those credentials"),
        "the error must name the real problem; got: {rendered}"
    );

    stub.await.expect("stub task joins");
}

#[tokio::test]
async fn a_missing_session_is_still_401() {
    // The 400 above is scoped to provider-credential rejection. An actually
    // unauthenticated caller must still get 401 — that is the signal the
    // interceptor is right to sign out on.
    let (base_url, stub) = stub_once("HTTP/1.1 200 OK", r#"{"id":"i123456"}"#).await;
    let (router, _) = setup(base_url).await;

    let response = AxumTestRequest::post("/api/providers/intervals_icu/link-credentials")
        .json(&json!({ "athlete_id": "i123456", "api_key": "some-key" }))
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
    stub.abort();
}

#[tokio::test]
async fn accepted_api_key_links_the_account_and_echoes_the_athlete() {
    // The happy path, end to end through the corrected Basic auth: the stub
    // accepts, the handler persists, and the response carries the real athlete
    // identity a returns-empty stub could not produce.
    let (base_url, stub) = stub_once(
        "HTTP/1.1 200 OK",
        r#"{"id":"i123456","name":"Test Athlete"}"#,
    )
    .await;
    let (router, auth) = setup(base_url).await;

    let response = AxumTestRequest::post("/api/providers/intervals_icu/link-credentials")
        .header("authorization", &auth)
        .json(&json!({ "athlete_id": "i123456", "api_key": "good-key" }))
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body: serde_json::Value = response.json();
    assert_eq!(body["status"], "connected");
    assert_eq!(body["provider"], "intervals_icu");
    assert_eq!(body["athlete"]["id"], "i123456");
    assert_eq!(body["athlete"]["name"], "Test Athlete");

    // The link call must have carried the literal `API_KEY` Basic username —
    // the athlete id belongs in the path only.
    let head = stub.await.expect("stub task joins");
    assert!(
        head.starts_with("GET /api/v1/athlete/i123456 "),
        "athlete id addresses the path; got head: {head}"
    );
    assert!(
        !head.contains("i123456:good-key"),
        "the athlete id must never appear in the credential pair"
    );
}
