// ABOUTME: Integration test for the Google Cloud Logging structured-JSON formatter.
// ABOUTME: Verifies emitted JSON matches Cloud Logging's LogEntry spec so alerts fire.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Shape tests for `GcpFormatter`: verify severity mapping, label placement,
//! structured-field flattening, and RFC 3339 timestamps.

use std::io::Write;
use std::sync::{Arc, Mutex};

use chrono::DateTime;
use pierre_mcp_server::logging::gcp::GcpFormatter;
use serde_json::Value;
use tracing::subscriber::with_default;
use tracing::{error, info, warn};
use tracing_subscriber::fmt::MakeWriter;

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
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("buffer lock").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Buffer {
    type Writer = Buffer;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn make_subscriber(
    buffer: Buffer,
    include_source_location: bool,
) -> impl tracing::Subscriber + Send + Sync {
    use tracing_subscriber::{fmt, layer::SubscriberExt, Registry};

    let layer = fmt::layer()
        .with_writer(buffer)
        .event_format(GcpFormatter::new(
            "pierre-mcp-server",
            "0.0.0-test",
            "development",
            include_source_location,
        ));

    Registry::default().with(layer)
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
fn single_event_emits_exactly_one_line() {
    let buf = Buffer::default();
    let subscriber = make_subscriber(buf.clone(), false);

    with_default(subscriber, || {
        for i in 0..5 {
            info!(i, "tick");
        }
    });

    assert_eq!(buf.lines().len(), 5);
}
