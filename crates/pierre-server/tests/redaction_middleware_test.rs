// ABOUTME: Integration tests for the installed PII redaction middleware layer
// ABOUTME: Asserts OAuth codes, tokens and addresses never survive into the logged request line
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Integration tests for `redaction_middleware`.
//!
//! The layer attaches a `RedactedRequestLine` to every request; the tower-http
//! `TraceLayer` span and `response_failure_log_middleware` both log that
//! instead of the raw URI. These tests drive the layer through a real router
//! and assert on the line it produces — an `OAuth` callback must reach a log
//! sink with its authorization code gone, not merely with the route named.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use axum::body::to_bytes;
use axum::body::Body;
use axum::http::{Request as HttpRequest, StatusCode};
use axum::routing::get;
use axum::{middleware, Extension, Router};
use http::Uri;
use pierre_middleware::redaction::{redact_query, redacted_request_line, redaction_middleware};
use pierre_middleware::redaction::{RedactedRequestLine, RedactionConfig};
use tower::ServiceExt;

/// Echoes the request line the layer attached, so the test reads exactly what
/// a log sink downstream of the layer would read.
async fn echo_line(Extension(line): Extension<RedactedRequestLine>) -> String {
    line.0
}

fn app(config: RedactionConfig) -> Router {
    Router::new()
        .route("/api/oauth/callback/strava", get(echo_line))
        .route("/api/auth/reset", get(echo_line))
        .route("/api/notifications", get(echo_line))
        .layer(middleware::from_fn_with_state(
            Arc::new(config),
            redaction_middleware,
        ))
}

async fn line_for(config: RedactionConfig, uri: &str) -> String {
    let request = HttpRequest::builder().uri(uri).body(Body::empty()).unwrap();
    let response = app(config).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn oauth_callback_query_reaches_the_log_without_its_code_or_state() {
    let line = line_for(
        RedactionConfig::default(),
        "/api/oauth/callback/strava?code=4b2c8fdeadbeef&state=xyz789&scope=read&email=coach@dravr.ai&contact=coach@dravr.ai",
    )
    .await;

    assert_eq!(
        line,
        "/api/oauth/callback/strava?code=[REDACTED]&state=[REDACTED]&scope=read&email=[REDACTED]&contact=c***@d***.ai",
        "the route and the harmless scope stay readable; the code, the state \
         and the address do not"
    );
    assert!(
        !line.contains("4b2c8fdeadbeef"),
        "the authorization code must not survive into the request line: {line}"
    );
    assert!(
        !line.contains("xyz789"),
        "the OAuth state must not survive into the request line: {line}"
    );
    assert!(
        !line.contains("coach@dravr.ai"),
        "no full address may survive into the request line: {line}"
    );
}

#[tokio::test]
async fn reset_token_is_replaced_and_the_route_is_kept() {
    let line = line_for(
        RedactionConfig::default(),
        "/api/auth/reset?token=eyJhbGciOi.payload.signature&user=42",
    )
    .await;

    assert_eq!(line, "/api/auth/reset?token=[REDACTED]&user=42");
}

#[tokio::test]
async fn a_query_free_request_line_is_just_the_path() {
    let line = line_for(RedactionConfig::default(), "/api/notifications").await;
    assert_eq!(line, "/api/notifications");
}

#[tokio::test]
async fn a_disabled_config_leaves_the_query_untouched() {
    // Local development reads its own logs; `logging.redact_pii=false` is the
    // switch for that and the layer must honour it rather than redact anyway.
    let line = line_for(
        RedactionConfig::new(false, "[REDACTED]".to_owned()),
        "/api/oauth/callback/strava?code=4b2c8fdeadbeef",
    )
    .await;

    assert_eq!(line, "/api/oauth/callback/strava?code=4b2c8fdeadbeef");
}

#[test]
fn redact_query_keeps_parameter_names_and_replaces_only_sensitive_values() {
    let config = RedactionConfig::default();

    assert_eq!(
        redact_query("provider=strava&code=abc123", &config),
        "provider=strava&code=[REDACTED]"
    );
    assert_eq!(
        redact_query("ACCESS_TOKEN=abc123", &config),
        "ACCESS_TOKEN=[REDACTED]",
        "parameter names are matched case-insensitively"
    );
    assert_eq!(
        redact_query("limit=25&offset=50", &config),
        "limit=25&offset=50",
        "pagination parameters are not secrets and stay legible"
    );
    assert_eq!(
        redact_query("flag", &config),
        "flag",
        "a valueless parameter survives"
    );
}

#[test]
fn redacted_request_line_composes_path_and_redacted_query() {
    let config = RedactionConfig::default();

    let uri: Uri = "/api/oauth/callback/fitbit?code=secret&scope=activity"
        .parse()
        .unwrap();
    assert_eq!(
        redacted_request_line(&uri, &config),
        "/api/oauth/callback/fitbit?code=[REDACTED]&scope=activity"
    );

    let bare: Uri = "/health".parse().unwrap();
    assert_eq!(redacted_request_line(&bare, &config), "/health");
}
