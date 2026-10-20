// ABOUTME: Integration tests for the single HTTP failure-logging middleware
// ABOUTME: Asserts endpoint-enriched ERROR alerts and WARN-not-ERROR for Retry-After backpressure
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Integration tests for `response_failure_log_middleware`.
//!
//! These pin the two behaviours that make a Sciotte backpressure storm both
//! non-paging and debuggable:
//!
//! * **Fix A** — a `503` carrying `Retry-After` is graceful load-shedding, so it
//!   logs at `WARN` (which `dravr-tronc` does not forward to `#dev-dravr-errors`)
//!   rather than `ERROR`. A `503` *without* `Retry-After` (e.g. the LLM-unhealthy
//!   readiness gate) still logs at `ERROR` — the downgrade is precise, not a
//!   blanket silencing of every 503.
//! * **Fix B** — every logged failure carries the `http_method` + `http_path`
//!   of the request that produced it, so the operator alert names the endpoint.
//!   The endpoint is the redacted request line `redaction_middleware` attaches,
//!   so a failing `OAuth` callback is named without its authorization code.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::HashMap;
use std::fmt::Debug as FmtDebug;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{header, Request as HttpRequest, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{middleware, Router};
use pierre_middleware::redaction::{redaction_middleware, RedactionConfig};
use pierre_middleware::response_failure_log_middleware;
use tower::ServiceExt;
use tracing::field::{Field, Visit};
use tracing::subscriber::DefaultGuard;
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

#[derive(Clone, Debug)]
struct CapturedEvent {
    level: tracing::Level,
    message: String,
    fields: HashMap<String, String>,
}

impl CapturedEvent {
    fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }
}

#[derive(Clone, Default)]
struct CaptureLayer {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

impl CaptureLayer {
    fn events(&self) -> Arc<Mutex<Vec<CapturedEvent>>> {
        Arc::clone(&self.events)
    }
}

/// Records every field of an event as a string so the test can assert both the
/// message and the structured `http_method` / `http_path` / `http_status`.
#[derive(Default)]
struct FieldVisitor {
    message: String,
    fields: HashMap<String, String>,
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn FmtDebug) {
        let rendered = format!("{value:?}");
        if field.name() == "message" {
            self.message.clone_from(&rendered);
        }
        self.fields.insert(field.name().to_owned(), rendered);
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().to_owned(), value.to_owned());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields
            .insert(field.name().to_owned(), value.to_string());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields
            .insert(field.name().to_owned(), value.to_string());
    }
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        self.events.lock().unwrap().push(CapturedEvent {
            level: *event.metadata().level(),
            message: visitor.message,
            fields: visitor.fields,
        });
    }
}

fn setup_capture() -> (Arc<Mutex<Vec<CapturedEvent>>>, DefaultGuard) {
    let capture = CaptureLayer::default();
    let events = capture.events();
    let guard = tracing_subscriber::registry().with(capture).set_default();
    (events, guard)
}

/// Backpressure shed: 503 that advertises a `Retry-After` for the client.
async fn shed_handler() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::RETRY_AFTER, "5")],
        "sciotte_busy",
    )
        .into_response()
}

/// Genuine dependency-down 503 with no retry hint (e.g. LLM readiness gate).
async fn unavailable_no_hint_handler() -> Response {
    (StatusCode::SERVICE_UNAVAILABLE, "llm down").into_response()
}

/// Genuine server fault.
async fn boom_handler() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "boom").into_response()
}

async fn ok_handler() -> &'static str {
    "ok"
}

fn app() -> Router {
    Router::new()
        .route("/api/providers/sciotte/login", get(shed_handler))
        .route("/ready", get(unavailable_no_hint_handler))
        .route("/boom", get(boom_handler))
        .route("/ok", get(ok_handler))
        .route("/api/oauth/callback/strava", get(boom_handler))
        .layer(middleware::from_fn(response_failure_log_middleware))
        // Mirrors the server's layer order: redaction is applied outside the
        // failure logger, so the log-safe request line is on the request by the
        // time the logger reads the endpoint off it.
        .layer(middleware::from_fn_with_state(
            Arc::new(RedactionConfig::default()),
            redaction_middleware,
        ))
}

async fn call(path: &str) -> Response {
    let request = HttpRequest::builder()
        .uri(path)
        .body(Body::empty())
        .unwrap();
    app().oneshot(request).await.unwrap()
}

#[tokio::test]
async fn retry_after_503_logs_warn_with_endpoint_and_never_errors() {
    let (events, guard) = setup_capture();

    let response = call("/api/providers/sciotte/login").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let snapshot = events.lock().unwrap().clone();
    drop(guard);

    // Fix A: the shed is logged, but at WARN — never at ERROR — so the ops
    // channel (`ERROR`-fed) is not paged for designed backpressure.
    let errored = snapshot.iter().any(|e| e.level == tracing::Level::ERROR);
    assert!(
        !errored,
        "a Retry-After 503 must not emit any ERROR event; captured: {snapshot:?}"
    );

    let warn = snapshot
        .iter()
        .find(|e| e.level == tracing::Level::WARN)
        .unwrap_or_else(|| panic!("expected a WARN shed event; captured: {snapshot:?}"));

    // Fix B: the event names the endpoint + method that shed.
    assert_eq!(
        warn.field("http_path"),
        Some("/api/providers/sciotte/login"),
        "shed WARN must carry the endpoint; captured: {snapshot:?}"
    );
    assert_eq!(warn.field("http_method"), Some("GET"));
    assert_eq!(warn.field("http_status"), Some("503"));
    assert_eq!(warn.field("retry_after_secs"), Some("5"));
}

#[tokio::test]
async fn plain_503_without_retry_after_still_errors() {
    let (events, guard) = setup_capture();

    let response = call("/ready").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let snapshot = events.lock().unwrap().clone();
    drop(guard);

    // Precision guard: the WARN downgrade is gated on Retry-After, so a bare
    // dependency-down 503 keeps paging.
    let error = snapshot
        .iter()
        .find(|e| e.level == tracing::Level::ERROR && e.message.contains("response failed"))
        .unwrap_or_else(|| panic!("bare 503 must still ERROR; captured: {snapshot:?}"));
    assert_eq!(error.field("http_path"), Some("/ready"));
    assert_eq!(error.field("http_status"), Some("503"));
    assert!(
        snapshot.iter().all(|e| e.level != tracing::Level::WARN),
        "a 503 without Retry-After must not be downgraded to WARN; captured: {snapshot:?}"
    );
}

#[tokio::test]
async fn server_error_logs_error_with_endpoint() {
    let (events, guard) = setup_capture();

    let response = call("/boom").await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let snapshot = events.lock().unwrap().clone();
    drop(guard);

    let error = snapshot
        .iter()
        .find(|e| e.level == tracing::Level::ERROR && e.message.contains("response failed"))
        .unwrap_or_else(|| panic!("a 500 must ERROR; captured: {snapshot:?}"));

    // Fix B: the endpoint that produced the 500 is on the alert.
    assert_eq!(error.field("http_path"), Some("/boom"));
    assert_eq!(error.field("http_method"), Some("GET"));
    assert_eq!(error.field("http_status"), Some("500"));
}

#[tokio::test]
async fn failure_on_an_oauth_callback_names_the_route_without_the_code() {
    let (events, guard) = setup_capture();

    let response = call("/api/oauth/callback/strava?code=4b2c8fdeadbeef&state=xyz789").await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let snapshot = events.lock().unwrap().clone();
    drop(guard);

    let error = snapshot
        .iter()
        .find(|e| e.level == tracing::Level::ERROR && e.message.contains("response failed"))
        .unwrap_or_else(|| panic!("a 500 must ERROR; captured: {snapshot:?}"));

    assert_eq!(
        error.field("http_path"),
        Some("/api/oauth/callback/strava?code=[REDACTED]&state=[REDACTED]"),
        "the alert keeps the query shape but none of its values; captured: {snapshot:?}"
    );
    let rendered = format!("{snapshot:?}");
    assert!(
        !rendered.contains("4b2c8fdeadbeef") && !rendered.contains("xyz789"),
        "no captured event may carry the authorization code or state: {rendered}"
    );
}

#[tokio::test]
async fn successful_response_logs_no_failure_event() {
    let (events, guard) = setup_capture();

    let response = call("/ok").await;
    assert_eq!(response.status(), StatusCode::OK);

    let snapshot = events.lock().unwrap().clone();
    drop(guard);

    assert!(
        snapshot.is_empty(),
        "a 2xx must not emit any failure event; captured: {snapshot:?}"
    );
}
