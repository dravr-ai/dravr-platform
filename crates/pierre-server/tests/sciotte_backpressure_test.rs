// ABOUTME: Regression tests pinning that a sciotte scraper load-shed is backpressure, not a system failure
// ABOUTME: Asserts the 503 + Retry-After answer and that no operator alert or sync.failed event is emitted
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The dedicated `dravr-sciotte` service sheds load by design when its Chrome
//! concurrency budget is saturated: `503` with
//! `{"error":"scraper_busy","reason":…,"retry_after_secs":N}` and a
//! `Retry-After` header (its `busy_response`). That body carries no login
//! `status` field, so before this fix it fell into `parse_login_outcome`'s
//! catch-all and became an `AppError::internal`, which every login handler
//! routed through `report_login_system_failure` — one `error!` forwarded to the
//! `#dev-dravr-errors` Slack channel *plus* one `sync.failed` business event
//! **per shed request**, precisely while the service was saturated. The
//! `retry_after_secs` the service had computed was discarded.
//!
//! These tests pin the two halves of the fix: the classifier recognises a shed
//! before any status match, and the login routes answer it with the service's
//! own `503` + `Retry-After` while emitting neither the operator alert nor the
//! business-failure event. A genuine failure still does both.

use std::collections::HashMap;
use std::fmt::Debug as FmtDebug;
use std::sync::{Arc, Mutex};

use axum::body::to_bytes;
use axum::http::{header, StatusCode};
use pierre_core::errors::AppError;
use pierre_providers::sciotte_remote::{backpressure_error, shed_retry_after_secs};
use pierre_routes_auth::{friendly_login_failure_message, login_failure_response};
use serde_json::{json, Value};
use tracing::field::{Field, Visit};
use tracing::subscriber::DefaultGuard;
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;
use uuid::Uuid;

/// The service's shed body, verbatim from its `busy_response`.
fn shed_body(retry_after_secs: u64) -> Value {
    json!({
        "error": "scraper_busy",
        "reason": "all chrome permits in use",
        "retry_after_secs": retry_after_secs,
    })
}

// ── Capture harness (same shape as `response_failure_log_test`) ─────────────

#[derive(Clone, Debug)]
struct CapturedEvent {
    level: tracing::Level,
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

#[derive(Default)]
struct FieldVisitor {
    fields: HashMap<String, String>,
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn FmtDebug) {
        self.fields
            .insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().to_owned(), value.to_owned());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
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
            fields: visitor.fields,
        });
    }
}

fn setup_capture() -> (Arc<Mutex<Vec<CapturedEvent>>>, DefaultGuard) {
    let capture = CaptureLayer::default();
    let events = Arc::clone(&capture.events);
    let guard = tracing_subscriber::registry().with(capture).set_default();
    (events, guard)
}

fn error_count(events: &Arc<Mutex<Vec<CapturedEvent>>>) -> usize {
    events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| e.level == tracing::Level::ERROR)
        .count()
}

fn sync_failed_count(events: &Arc<Mutex<Vec<CapturedEvent>>>) -> usize {
    events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| e.field("event") == Some("sync.failed"))
        .count()
}

// ── Classification: a shed is not a system failure ──────────────────────────

#[test]
fn a_scraper_busy_503_is_classified_as_retryable_backpressure() {
    let error = backpressure_error(StatusCode::SERVICE_UNAVAILABLE, &shed_body(12))
        .expect("the service's documented shed response must be recognised");

    assert_eq!(
        error.http_status(),
        503,
        "a shed is service-unavailable, not a 500 fault"
    );
    assert_eq!(
        shed_retry_after_secs(&error),
        Some(12),
        "the wait the service computed must survive the mapping instead of being discarded"
    );
}

#[test]
fn a_scraper_busy_body_is_backpressure_whatever_the_status() {
    let error = backpressure_error(StatusCode::TOO_MANY_REQUESTS, &shed_body(7))
        .expect("the `scraper_busy` marker is enough on its own");
    assert_eq!(shed_retry_after_secs(&error), Some(7));
}

#[test]
fn a_shed_without_a_wait_still_advertises_one() {
    let error = backpressure_error(
        StatusCode::SERVICE_UNAVAILABLE,
        &json!({ "error": "scraper_busy", "reason": "queue full" }),
    )
    .expect("a shed missing its hint is still a shed");
    let retry_after = shed_retry_after_secs(&error)
        .expect("a shed must always advertise a wait so the caller knows to retry");
    assert!(
        (1..=300).contains(&retry_after),
        "the fallback wait must be a usable retry window, got {retry_after}s"
    );
}

#[test]
fn a_genuine_failure_is_not_backpressure() {
    assert!(
        backpressure_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &json!({ "error": "chrome crashed" }),
        )
        .is_none(),
        "a 500 with no shed marker is a real fault and must keep alerting"
    );
    assert!(
        backpressure_error(StatusCode::UNAUTHORIZED, &json!({ "error": "bad api key" })).is_none(),
        "a misconfigured DRAVR_SCIOTTE_API_KEY must not masquerade as backpressure"
    );
    assert!(
        shed_retry_after_secs(&AppError::internal("chrome crashed")).is_none(),
        "an internal error advertises no retry window"
    );
}

// ── Route layer: the shed answer, and what it must not emit ─────────────────

#[tokio::test]
async fn a_login_shed_answers_503_with_retry_after_and_pages_nobody() {
    let shed = backpressure_error(StatusCode::SERVICE_UNAVAILABLE, &shed_body(45))
        .expect("shed classified");

    let (events, guard) = setup_capture();
    let response = login_failure_response(
        Uuid::new_v4(),
        Uuid::new_v4(),
        "sciotte",
        "credential_login",
        &shed,
    )
    .expect("a shed is answered, not raised as a system failure");

    assert_eq!(
        error_count(&events),
        0,
        "designed load-shedding must not emit the error! that dravr-tronc forwards \
         to #dev-dravr-errors — one page per shed request is the storm this fixes"
    );
    assert_eq!(
        sync_failed_count(&events),
        0,
        "a shed is not a business failure, so no sync.failed analytics event"
    );
    drop(guard);

    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "the athlete gets the service's own 503"
    );
    assert_eq!(
        response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok()),
        Some("45"),
        "the Retry-After header carries the service's wait — it is also what \
         response_failure_log_middleware keys on to log this at WARN, not ERROR"
    );

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read shed body");
    let body: Value = serde_json::from_slice(&body).expect("shed body is JSON");
    assert_eq!(
        body["message"],
        Value::from(friendly_login_failure_message("sciotte")),
        "the login modal renders `message`, so it must be the friendly copy"
    );
    assert_eq!(
        body["details"]["retry_after_secs"],
        Value::from(45),
        "the wait is machine-readable for the client too"
    );
}

#[tokio::test]
async fn a_login_system_failure_still_alerts_operators() {
    let failure = AppError::internal("browser error: Failed to launch browser");

    let (events, guard) = setup_capture();
    let error = login_failure_response(
        Uuid::new_v4(),
        Uuid::new_v4(),
        "sciotte_garmin",
        "otp",
        &failure,
    )
    .expect_err("a real fault is still a system failure");

    assert_eq!(
        error_count(&events),
        1,
        "a genuine fault must still page operators exactly once"
    );
    assert_eq!(
        sync_failed_count(&events),
        1,
        "a genuine fault is still a business failure worth the analytics event"
    );
    drop(guard);

    assert_eq!(
        error.sanitized_message(),
        friendly_login_failure_message("sciotte_garmin"),
        "the athlete still gets friendly, provider-aware copy"
    );
    assert_eq!(error.http_status(), 400);
}
