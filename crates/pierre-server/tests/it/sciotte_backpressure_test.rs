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
//! `status` field. Read as a system failure, every shed request cost one
//! `error!` forwarded to the `#dev-dravr-errors` Slack channel *plus* one
//! `sync.failed` business event, precisely while the service was saturated,
//! and the `retry_after_secs` the service had computed was discarded.
//!
//! The sciotte client names the shed (pinned in dravr-sciotte's own client
//! tests) and `sciotte_error` maps it (pinned in that module's unit tests).
//! These tests pin the route layer: the login routes answer a mapped shed with
//! the service's own `503` + `Retry-After` while emitting neither the operator
//! alert nor the business-failure event. A genuine failure still does both,
//! and what it logs names the answer's marker without its body.

use std::collections::HashMap;
use std::fmt::Debug as FmtDebug;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::to_bytes;
use axum::http::{header, StatusCode};
use dravr_sciotte::client::{ClientError, Exchange, Operation, RequestId, ServiceError};
use pierre_core::errors::AppError;
use pierre_providers::sciotte_error::to_app_error;
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

/// One attempt of `operation`, as the sciotte client reports it.
fn exchange(operation: Operation) -> Exchange {
    Exchange {
        service: "sciotte".to_owned(),
        operation: operation.as_str().to_owned(),
        request_id: RequestId::mint(),
        elapsed: Duration::from_millis(8),
        resent: false,
    }
}

/// The platform error for the service shedding a login step, as the sciotte
/// client reports its `503` + `{"error":"scraper_busy","retry_after_secs":N}`.
fn login_shed(retry_after_secs: u64) -> AppError {
    to_app_error(ClientError::Service(ServiceError::Shed {
        exchange: exchange(Operation::Login).into(),
        status: StatusCode::SERVICE_UNAVAILABLE,
        retry_after_secs,
        reason: Some("all chrome permits in use".to_owned()),
    }))
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

// ── Route layer: the shed answer, and what it must not emit ─────────────────

#[tokio::test]
async fn a_login_shed_answers_503_with_retry_after_and_pages_nobody() {
    let shed = login_shed(45);

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

#[tokio::test]
async fn a_login_answer_with_no_status_alerts_with_its_marker_and_not_its_body() {
    // The identity-token gate's 401: no login status, and a body an operator
    // log must not carry beyond the marker that says what refused the call.
    let failure = to_app_error(ClientError::Unexpected {
        exchange: exchange(Operation::Login).into(),
        status: StatusCode::UNAUTHORIZED,
        body: json!({
            "error": { "type": "unauthorized", "message": "audience mismatch for token eyJhbGci" },
        }),
    });

    let (events, guard) = setup_capture();
    login_failure_response(
        Uuid::new_v4(),
        Uuid::new_v4(),
        "sciotte",
        "credential_login",
        &failure,
    )
    .expect_err("an answer with no login status is a system failure");

    let reasons: Vec<String> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| event.field("reason").map(str::to_owned))
        .collect();
    drop(guard);

    assert_eq!(
        reasons.len(),
        2,
        "the operator alert and the sync.failed event each carry the reason: {reasons:?}"
    );
    for reason in &reasons {
        assert!(
            reason.contains("HTTP 401 Unauthorized") && reason.contains("marker: unauthorized"),
            "the alert says what answered: {reason}"
        );
        assert!(
            reason.contains("DRAVR_SCIOTTE_AUDIENCE"),
            "and where to look: {reason}"
        );
        assert!(
            !reason.contains("eyJhbGci") && !reason.contains("audience mismatch"),
            "the body stays out of the log and the business event: {reason}"
        );
    }
}
