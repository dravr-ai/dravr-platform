// ABOUTME: Integration test for the Google Cloud Logging structured-JSON formatter.
// ABOUTME: Verifies emitted JSON matches Cloud Logging's LogEntry spec so alerts fire.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Shape tests for `GcpFormatter`: verify severity mapping, label placement,
//! structured-field flattening, and RFC 3339 timestamps.

use crate::common;

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::Request;
use chrono::DateTime;
use pierre_logging::gcp::GcpFormatter;
use pierre_logging::span_fields::SpanFieldStorage;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use serde_json::Value;
use tower::ServiceExt;
use tracing::field::Empty;
use tracing::subscriber::{set_default, with_default};
use tracing::{error, info, info_span, warn, Subscriber};
use tracing_subscriber::fmt::{self, MakeWriter};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::Registry;

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    fn lines(&self) -> Vec<Value> {
        let bytes = self.0.lock().expect("buffer lock").clone();
        String::from_utf8(bytes)
            .expect("valid utf8")
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_str::<Value>(l).expect("valid json line"))
            .collect()
    }
}

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("buffer lock").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Buffer {
    type Writer = Self;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn make_subscriber(buffer: Buffer, include_source_location: bool) -> impl Subscriber + Send + Sync {
    let layer = fmt::layer()
        .with_writer(buffer)
        .event_format(GcpFormatter::new(
            "pierre-mcp-server",
            "0.0.0-test",
            "development",
            include_source_location,
        ));

    // Mirror the production layer stack: SpanFieldStorage captures span field
    // values so the formatter can propagate them onto child events.
    Registry::default().with(SpanFieldStorage).with(layer)
}

#[test]
fn warn_event_maps_to_warning_severity() {
    let buf = Buffer::default();
    let subscriber = make_subscriber(buf.clone(), false);

    with_default(subscriber, || {
        warn!("something noteworthy");
    });

    let lines = buf.lines();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["severity"], "WARNING");
    assert_eq!(lines[0]["message"], "something noteworthy");
}

#[test]
fn error_event_maps_to_error_severity() {
    let buf = Buffer::default();
    let subscriber = make_subscriber(buf.clone(), false);

    with_default(subscriber, || {
        error!("boom");
    });

    let lines = buf.lines();
    assert_eq!(lines[0]["severity"], "ERROR");
}

#[test]
fn info_event_has_rfc3339_time_and_labels() {
    let buf = Buffer::default();
    let subscriber = make_subscriber(buf.clone(), false);

    with_default(subscriber, || {
        info!("hello");
    });

    let entry = &buf.lines()[0];
    let time = entry["time"].as_str().expect("time is string");
    DateTime::parse_from_rfc3339(time).expect("time is RFC 3339");

    let labels = &entry["logging.googleapis.com/labels"];
    assert_eq!(labels["service.name"], "pierre-mcp-server");
    assert_eq!(labels["service.version"], "0.0.0-test");
    assert_eq!(labels["environment"], "development");
    assert!(labels["rust.target"].as_str().is_some());
}

#[test]
fn structured_fields_appear_at_top_level() {
    let buf = Buffer::default();
    let subscriber = make_subscriber(buf.clone(), false);

    with_default(subscriber, || {
        info!(tenant_id = "abc-123", user_count = 42, "processed");
    });

    let entry = &buf.lines()[0];
    assert_eq!(entry["tenant_id"], "abc-123");
    assert_eq!(entry["user_count"], 42);
    assert_eq!(entry["message"], "processed");
}

#[test]
fn source_location_emitted_when_enabled() {
    let buf = Buffer::default();
    let subscriber = make_subscriber(buf.clone(), true);

    with_default(subscriber, || {
        info!("traced");
    });

    let entry = &buf.lines()[0];
    let loc = &entry["logging.googleapis.com/sourceLocation"];
    let file = loc["file"].as_str().expect("file present");
    assert!(file.ends_with("logging_gcp_test.rs"));
    assert!(loc["line"].as_str().is_some());
}

#[test]
fn source_location_omitted_when_disabled() {
    let buf = Buffer::default();
    let subscriber = make_subscriber(buf.clone(), false);

    with_default(subscriber, || {
        info!("no loc");
    });

    let entry = &buf.lines()[0];
    assert!(entry.get("logging.googleapis.com/sourceLocation").is_none());
}

#[test]
fn multiple_events_emit_one_line_each() {
    let buf = Buffer::default();
    let subscriber = make_subscriber(buf.clone(), false);

    with_default(subscriber, || {
        for i in 0..5 {
            info!(i, "tick");
        }
    });

    assert_eq!(buf.lines().len(), 5);
}

#[test]
fn span_fields_propagate_onto_child_events() {
    let buf = Buffer::default();
    let subscriber = make_subscriber(buf.clone(), false);

    with_default(subscriber, || {
        let span = info_span!(
            "turn",
            user_id = "user-1",
            tenant_id = "tenant-9",
            conversation_id = "conv-7"
        );
        let _guard = span.enter();
        // An event with no identifiers of its own — exactly the case that
        // produced context-free "persona conformance violation" lines.
        warn!("persona conformance violation");
    });

    let entry = &buf.lines()[0];
    assert_eq!(entry["message"], "persona conformance violation");
    assert_eq!(entry["user_id"], "user-1");
    assert_eq!(entry["tenant_id"], "tenant-9");
    assert_eq!(entry["conversation_id"], "conv-7");
}

#[test]
fn event_fields_win_over_span_fields() {
    let buf = Buffer::default();
    let subscriber = make_subscriber(buf.clone(), false);

    with_default(subscriber, || {
        let span = info_span!("turn", tenant_id = "from-span");
        let _guard = span.enter();
        info!(tenant_id = "from-event", "override");
    });

    // The event's own field takes precedence over the inherited span field.
    assert_eq!(buf.lines()[0]["tenant_id"], "from-event");
}

#[test]
fn nearest_span_wins_for_nested_spans() {
    let buf = Buffer::default();
    let subscriber = make_subscriber(buf.clone(), false);

    with_default(subscriber, || {
        let outer = info_span!("outer", scope = "root", tenant_id = "outer-tenant");
        let _o = outer.enter();
        let inner = info_span!("inner", scope = "leaf");
        let _i = inner.enter();
        info!("nested");
    });

    let entry = &buf.lines()[0];
    // Leaf span overrides the ancestor's `scope`; the ancestor still
    // contributes `tenant_id`, which the leaf does not set.
    assert_eq!(entry["scope"], "leaf");
    assert_eq!(entry["tenant_id"], "outer-tenant");
}

#[test]
fn deferred_span_field_recorded_later_propagates() {
    let buf = Buffer::default();
    let subscriber = make_subscriber(buf.clone(), false);

    with_default(subscriber, || {
        // agent_id/group_id are declared Empty at span creation and recorded
        // mid-turn once the conversation record resolves.
        let span = info_span!("turn", user_id = "user-2", agent_id = Empty);
        span.record("agent_id", "marathon-coach");
        let _guard = span.enter();
        warn!("something happened mid-turn");
    });

    let entry = &buf.lines()[0];
    assert_eq!(entry["user_id"], "user-2");
    assert_eq!(entry["agent_id"], "marathon-coach");
}

/// Subscriber whose formatter links lines to the request trace under `project`.
fn make_trace_subscriber(buffer: Buffer, project: &str) -> impl Subscriber + Send + Sync {
    let layer = fmt::layer().with_writer(buffer).event_format(
        GcpFormatter::new("pierre-mcp-server", "0.0.0-test", "development", false)
            .with_trace_project(project.to_owned()),
    );
    Registry::default().with(SpanFieldStorage).with(layer)
}

const TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";

#[test]
fn request_trace_id_becomes_the_cloud_logging_trace() {
    let buf = Buffer::default();
    let subscriber = make_trace_subscriber(buf.clone(), "dravr-dev");

    with_default(subscriber, || {
        let span = info_span!(
            "request",
            cloud_trace_id = TRACE_ID,
            http_request_id = "req-1"
        );
        let _guard = span.enter();
        error!("handler failed");
    });

    let entry = &buf.lines()[0];
    assert_eq!(
        entry["logging.googleapis.com/trace"],
        format!("projects/dravr-dev/traces/{TRACE_ID}")
    );
    assert!(entry.get("cloud_trace_id").is_none());
    assert_eq!(entry["http_request_id"], "req-1");
}

#[test]
fn trace_id_stays_a_plain_field_without_a_project() {
    let buf = Buffer::default();
    let subscriber = make_subscriber(buf.clone(), false);

    with_default(subscriber, || {
        let span = info_span!("request", cloud_trace_id = TRACE_ID);
        let _guard = span.enter();
        info!("no project configured");
    });

    let entry = &buf.lines()[0];
    assert_eq!(entry["cloud_trace_id"], TRACE_ID);
    assert!(entry.get("logging.googleapis.com/trace").is_none());
}

#[test]
fn line_outside_a_request_carries_no_trace() {
    let buf = Buffer::default();
    let subscriber = make_trace_subscriber(buf.clone(), "dravr-dev");

    with_default(subscriber, || {
        info!("background sweep");
    });

    let entry = &buf.lines()[0];
    assert!(entry.get("logging.googleapis.com/trace").is_none());
}

/// The production HTTP stack serving one request under the GCP formatter. The
/// request-id middleware sits outside the trace layer, so the request span
/// carries the id the response returns; the inbound `traceparent` becomes the
/// line's Cloud Logging trace. A route-order regression drops both silently.
#[tokio::test]
async fn a_served_request_logs_its_request_id_and_inbound_trace() {
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let buf = Buffer::default();

    // An unrouted path: answered by the router's fallback inside every
    // layer, without a database round trip on another thread.
    let guard = set_default(make_trace_subscriber(buf.clone(), "dravr-dev"));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/no-such-route")
                .header("traceparent", format!("00-{TRACE_ID}-00f067aa0ba902b7-01"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    drop(guard);

    let request_id = response
        .headers()
        .get("x-request-id")
        .expect("the request-id middleware answers with the id")
        .to_str()
        .unwrap()
        .to_owned();
    let lines = buf.lines();
    let finished = lines
        .iter()
        .find(|line| line["message"] == "finished processing request")
        .unwrap_or_else(|| panic!("no response line was logged: {lines:?}"));
    assert_eq!(
        finished["http_request_id"],
        request_id.as_str(),
        "the request span must carry the id the response returned"
    );
    assert_eq!(
        finished["logging.googleapis.com/trace"],
        format!("projects/dravr-dev/traces/{TRACE_ID}"),
        "the request span must carry the trace Cloud Run stamped"
    );
}
