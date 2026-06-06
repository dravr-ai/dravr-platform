// ABOUTME: Integration test for the Google Cloud Logging structured-JSON formatter.
// ABOUTME: Verifies emitted JSON matches Cloud Logging's LogEntry spec so alerts fire.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Shape tests for `GcpFormatter`: verify severity mapping, label placement,
//! structured-field flattening, and RFC 3339 timestamps.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use chrono::DateTime;
use pierre_logging::gcp::GcpFormatter;
use pierre_logging::span_fields::SpanFieldStorage;
use serde_json::Value;
use tracing::field::Empty;
use tracing::subscriber::with_default;
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
        // coach_id/group_id are declared Empty at span creation and recorded
        // mid-turn once the conversation record resolves.
        let span = info_span!("turn", user_id = "user-2", coach_id = Empty);
        span.record("coach_id", "marathon-coach");
        let _guard = span.enter();
        warn!("something happened mid-turn");
    });

    let entry = &buf.lines()[0];
    assert_eq!(entry["user_id"], "user-2");
    assert_eq!(entry["coach_id"], "marathon-coach");
}
