// ABOUTME: Integration tests for the chromiumoxide notifier-sink filter that suppresses post-close WS resets
// ABOUTME: Verifies ResetWithoutClosingHandshake events are dropped at the Slack alert path while other chromiumoxide errors pass through
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::fmt::Debug as FmtDebug;
use std::sync::{Arc, Mutex};

use pierre_mcp_server::logging::ChromiumoxideNotifierFilter;
use tracing::field::{Field, Visit};
use tracing::subscriber::DefaultGuard;
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

#[derive(Clone, Debug)]
struct CapturedEvent {
    target: String,
    level: tracing::Level,
    message: String,
}

#[derive(Clone, Default)]
struct CaptureLayer {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

impl CaptureLayer {
    fn new() -> Self {
        Self::default()
    }

    fn events(&self) -> Arc<Mutex<Vec<CapturedEvent>>> {
        Arc::clone(&self.events)
    }
}

struct MessageVisitor {
    message: String,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn FmtDebug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.message);
        }
    }
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MessageVisitor {
            message: String::new(),
        };
        event.record(&mut visitor);
        self.events.lock().unwrap().push(CapturedEvent {
            target: event.metadata().target().to_owned(),
            level: *event.metadata().level(),
            message: visitor.message,
        });
    }
}

/// Install a scoped tracing subscriber that captures events the
/// `ChromiumoxideNotifierFilter` lets through. Returns the captured
/// events handle and the dispatcher guard. Drop the guard to detach
/// the subscriber from the current thread.
fn setup_capture() -> (Arc<Mutex<Vec<CapturedEvent>>>, DefaultGuard) {
    let capture = CaptureLayer::new();
    let events = capture.events();

    let subscriber =
        tracing_subscriber::registry().with(capture.with_filter(ChromiumoxideNotifierFilter));

    let guard = subscriber.set_default();
    (events, guard)
}

#[test]
fn reset_without_closing_handshake_is_suppressed() {
    let (events, guard) = setup_capture();

    tracing::error!(
        target: "chromiumoxide::handler",
        "WS Connection error: Ws(Protocol(ResetWithoutClosingHandshake))"
    );

    let snapshot = events.lock().unwrap().clone();
    drop(guard);

    let leaked = snapshot.iter().any(|e| {
        e.target == "chromiumoxide::handler"
            && e.level == tracing::Level::ERROR
            && e.message.contains("ResetWithoutClosingHandshake")
    });

    assert!(
        !leaked,
        "chromiumoxide handler ERROR mentioning ResetWithoutClosingHandshake \
         must be suppressed at the notifier sink; captured: {snapshot:?}"
    );
}

#[test]
fn other_chromiumoxide_errors_still_reach_notifier() {
    let (events, guard) = setup_capture();

    tracing::error!(
        target: "chromiumoxide::handler",
        "CDP message decode failed: invalid frame"
    );

    let snapshot = events.lock().unwrap().clone();
    drop(guard);

    let passed_through = snapshot.iter().any(|e| {
        e.target == "chromiumoxide::handler"
            && e.level == tracing::Level::ERROR
            && e.message.contains("CDP message decode failed")
    });

    assert!(
        passed_through,
        "non-reset chromiumoxide ERRORs must still reach the notifier; \
         captured: {snapshot:?}"
    );
}

#[test]
fn reset_token_in_unrelated_crate_is_not_suppressed() {
    let (events, guard) = setup_capture();

    // Target gate prevents false-positives across the codebase.
    tracing::error!(
        target: "my_crate::ws",
        "WebSocket error: ResetWithoutClosingHandshake"
    );

    let snapshot = events.lock().unwrap().clone();
    drop(guard);

    let unrelated_passed = snapshot
        .iter()
        .any(|e| e.target == "my_crate::ws" && e.message.contains("ResetWithoutClosingHandshake"));

    assert!(
        unrelated_passed,
        "ResetWithoutClosingHandshake from a non-chromiumoxide target must \
         pass through — the filter is target-scoped to avoid false positives; \
         captured: {snapshot:?}"
    );
}

#[test]
fn warn_level_chromiumoxide_event_is_not_suppressed() {
    let (events, guard) = setup_capture();

    // Level gate keeps non-ERROR visible.
    tracing::warn!(
        target: "chromiumoxide::handler",
        "Heads-up: ResetWithoutClosingHandshake observed during shutdown"
    );

    let snapshot = events.lock().unwrap().clone();
    drop(guard);

    let warn_passed = snapshot.iter().any(|e| {
        e.target == "chromiumoxide::handler"
            && e.level == tracing::Level::WARN
            && e.message.contains("ResetWithoutClosingHandshake")
    });

    assert!(
        warn_passed,
        "non-ERROR chromiumoxide events stay visible; only ERROR is suppressed; \
         captured: {snapshot:?}"
    );
}
