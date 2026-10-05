// ABOUTME: Tests for the CSRF validator and the layer that applies it to cookie sessions
// ABOUTME: Covers token verification, and that no unchecked cookie ever authenticates a request
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{to_bytes, Body};
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::Router;
use pierre_auth::security::cookies::auth_cookie_name;
use pierre_auth::security::csrf::CsrfTokenManager;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_middleware::csrf::validate_csrf_token;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

#[test]
fn test_csrf_validator_get_request() {
    let csrf_manager = Arc::new(CsrfTokenManager::default());
    let headers = HeaderMap::new();
    let user_id = Uuid::new_v4();

    // GET requests should not require CSRF token
    let result = validate_csrf_token(&headers, &Method::GET, user_id, &csrf_manager);

    assert!(result.is_ok(), "GET request should not require CSRF token");
}

#[test]
fn test_csrf_validator_post_without_token() {
    let csrf_manager = Arc::new(CsrfTokenManager::default());
    let headers = HeaderMap::new();
    let user_id = Uuid::new_v4();

    // POST without CSRF token should fail
    let result = validate_csrf_token(&headers, &Method::POST, user_id, &csrf_manager);

    assert!(
        result.is_err(),
        "POST request without CSRF token should fail"
    );
}

#[test]
fn test_csrf_validator_post_with_valid_token() -> anyhow::Result<()> {
    let csrf_manager = Arc::new(CsrfTokenManager::default());
    let user_id = Uuid::new_v4();
    let token = csrf_manager.generate_token(user_id)?;

    let mut headers = HeaderMap::new();
    headers.insert("X-CSRF-Token", token.parse()?);

    // POST with valid CSRF token should succeed
    let result = validate_csrf_token(&headers, &Method::POST, user_id, &csrf_manager);

    assert!(
        result.is_ok(),
        "POST request with valid CSRF token should succeed"
    );
    Ok(())
}

#[test]
fn test_csrf_validator_post_with_invalid_token() -> anyhow::Result<()> {
    let csrf_manager = Arc::new(CsrfTokenManager::default());
    let user_id = Uuid::new_v4();

    let mut headers = HeaderMap::new();
    headers.insert("X-CSRF-Token", "invalid_token".parse()?);

    // POST with invalid CSRF token should fail
    let result = validate_csrf_token(&headers, &Method::POST, user_id, &csrf_manager);

    assert!(
        result.is_err(),
        "POST request with invalid CSRF token should fail"
    );
    Ok(())
}

#[test]
fn test_csrf_validator_method_class_gating() {
    let csrf_manager = Arc::new(CsrfTokenManager::default());
    let headers = HeaderMap::new();
    let user_id = Uuid::new_v4();

    // State-changing methods reject empty-headers requests
    for method in [Method::POST, Method::PUT, Method::DELETE, Method::PATCH] {
        let result = validate_csrf_token(&headers, &method, user_id, &csrf_manager);
        assert!(
            result.is_err(),
            "{method} should require a CSRF token (got Ok)"
        );
    }

    // Read-only methods bypass the check entirely
    for method in [Method::GET, Method::HEAD, Method::OPTIONS] {
        let result = validate_csrf_token(&headers, &method, user_id, &csrf_manager);
        assert!(
            result.is_ok(),
            "{method} should not require a CSRF token (got Err)"
        );
    }
}

// ============================================================================
// The layer, end to end: no cookie authenticates a request it did not check
// ============================================================================

/// The served app, a signed-in athlete's session token, and a CSRF token
/// minted for that athlete.
async fn signed_in_app() -> (Router, Arc<ServerContext>, String, String) {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user, session) = common::create_test_tenant(&resources, "csrf-layer@example.com")
        .await
        .unwrap();
    let csrf = resources.auth.csrf_manager.generate_token(user.id).unwrap();
    (
        ProviderToolRouter::build_http_app(&resources),
        resources,
        session,
        csrf,
    )
}

/// `POST /api/keys`, a state-changing route behind the cookie session, with
/// the given extra headers.
async fn create_key(app: &Router, headers: &[(&str, &str)]) -> (StatusCode, Value) {
    post_json(app, "/api/keys", &json!({ "name": "csrf layer" }), headers).await
}

async fn post_json(
    app: &Router,
    uri: &str,
    body: &Value,
    headers: &[(&str, &str)],
) -> (StatusCode, Value) {
    send_json(app, Method::POST, uri, body, headers).await
}

async fn send_json(
    app: &Router,
    method: Method,
    uri: &str,
    body: &Value,
    headers: &[(&str, &str)],
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn test_cookie_session_needs_its_csrf_token_to_change_state() {
    let (app, _resources, session, csrf) = signed_in_app().await;
    let cookie = format!("{}={session}", auth_cookie_name());

    let (status, body) = create_key(&app, &[("cookie", &cookie)]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "no CSRF token: {body}");

    let (status, body) = create_key(&app, &[("cookie", &cookie), ("x-csrf-token", &csrf)]).await;
    assert_eq!(status, StatusCode::CREATED, "the app's own request: {body}");
}

/// A request carrying an `Authorization` header skips the CSRF check, so the
/// header must be what authenticates it. `PUT /api/user/profile` reads the
/// cookie before the header: left on the request, a valid session cookie
/// beside a junk header changed the athlete's profile with no CSRF token
/// (carnet#768).
#[tokio::test]
async fn test_a_bearer_header_does_not_let_the_cookie_skip_the_csrf_check() {
    let (app, _resources, session, _csrf) = signed_in_app().await;
    let cookie = format!("{}={session}", auth_cookie_name());
    let rename = json!({ "display_name": "Forged Name" });

    let (status, body) = send_json(
        &app,
        Method::PUT,
        "/api/user/profile",
        &rename,
        &[("cookie", &cookie), ("authorization", "Bearer not-a-token")],
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "the cookie must not authenticate a request that skipped the check: {body}"
    );

    // The header alone still authenticates, as a programmatic client's does.
    let bearer = format!("Bearer {session}");
    let (status, body) = send_json(
        &app,
        Method::PUT,
        "/api/user/profile",
        &rename,
        &[("authorization", &bearer)],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// A cookie the layer cannot read is removed, so the request is genuinely
/// unauthenticated: a route behind the session refuses it, and a public form
/// (forgot-password) is still served to a browser holding a stale cookie.
#[tokio::test]
async fn test_an_unreadable_cookie_is_removed_rather_than_trusted_or_refused() {
    let (app, _resources, _session, _csrf) = signed_in_app().await;
    let stale = format!("{}=not.a.jwt", auth_cookie_name());

    let (status, body) = create_key(&app, &[("cookie", &stale)]).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    let (status, body) = post_json(
        &app,
        "/api/auth/forgot-password",
        &json!({ "email": "csrf-layer@example.com" }),
        &[("cookie", &stale)],
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a stale cookie must not lock the browser out of a public form: {body}"
    );
}
