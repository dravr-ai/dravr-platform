// ABOUTME: Integration tests for the chromiumoxide notifier-sink filter that suppresses post-close WS resets
// ABOUTME: Verifies ResetWithoutClosingHandshake events are dropped at the Slack alert path while other chromiumoxide errors pass through
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::{Arc, Mutex};

use pierre_mcp_server::logging::ChromiumoxideNotifierFilter;
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

/// Layer that records every event it sees into a shared buffer.
/// Stand-in for the `ErrorNotificationLayer` so we can assert exactly
/// which events would have reached Slack.
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

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
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

/// Emit the four event shapes the filter must classify, capture what
/// reaches the notifier sink, and return a cloned event list. Runs
/// inside a scoped subscriber so the test does not race with
/// `LoggingConfig::init`'s global installation. The capture Arc is
/// held by the dispatcher for the lifetime of `_guard`; we clone the
/// events out of the Mutex rather than unwrapping the Arc because
/// `tracing::Dispatch` retains a reference to the layer set.
fn run_with_filter_and_capture() -> Vec<CapturedEvent> {
    let capture = CaptureLayer::new();
    let events = capture.events();

    let subscriber =
        tracing_subscriber::registry().with(capture.with_filter(ChromiumoxideNotifierFilter));

    let _guard = subscriber.set_default();

    // Case 1: the post-close noise pattern — must be suppressed.
    tracing::error!(
        target: "chromiumoxide::handler",
        "WS Connection error: Ws(Protocol(ResetWithoutClosingHandshake))"
    );

    // Case 2: a different chromiumoxide handler ERROR — must pass.
    tracing::error!(
        target: "chromiumoxide::handler",
        "CDP message decode failed: invalid frame"
    );

    // Case 3: an unrelated crate emitting ResetWithoutClosingHandshake —
    // must pass (target gate prevents false-positives across the codebase).
    tracing::error!(
        target: "my_crate::ws",
        "WebSocket error: ResetWithoutClosingHandshake"
    );

    // Case 4: a WARN at the chromiumoxide handler target mentioning the
    // token — must pass (level gate keeps non-ERROR visible).
    tracing::warn!(
        target: "chromiumoxide::handler",
        "Heads-up: ResetWithoutClosingHandshake observed during shutdown"
    );

    let snapshot = events.lock().unwrap().clone();
    drop(_guard);
    snapshot
}

#[test]
fn reset_without_closing_handshake_is_suppressed() {
    let events = run_with_filter_and_capture();

    let matched: Vec<_> = events
        .iter()
        .filter(|e| {
            e.target == "chromiumoxide::handler"
                && e.level == tracing::Level::ERROR
                && e.message.contains("ResetWithoutClosingHandshake")
        })
        .collect();

    assert!(
        matched.is_empty(),
        "chromiumoxide handler ERROR mentioning ResetWithoutClosingHandshake \
         must be suppressed at the notifier sink; got {} match(es): {:?}",
        matched.len(),
        matched
            .iter()
            .map(|e| e.message.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn other_chromiumoxide_errors_still_reach_notifier() {
    let events = run_with_filter_and_capture();

    let passed_through = events.iter().any(|e| {
        e.target == "chromiumoxide::handler"
            && e.level == tracing::Level::ERROR
            && e.message.contains("CDP message decode failed")
    });

    assert!(
        passed_through,
        "non-reset chromiumoxide ERRORs must still reach the notifier; \
         captured events: {:?}",
        events
            .iter()
            .map(|e| (e.target.as_str(), e.level, e.message.as_str()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn reset_token_in_unrelated_crate_is_not_suppressed() {
    let events = run_with_filter_and_capture();

    let unrelated_passed = events
        .iter()
        .any(|e| e.target == "my_crate::ws" && e.message.contains("ResetWithoutClosingHandshake"));

    assert!(
        unrelated_passed,
        "ResetWithoutClosingHandshake from a non-chromiumoxide target must \
         pass through — the filter is target-scoped to avoid false positives"
    );
}

#[test]
fn warn_level_chromiumoxide_event_is_not_suppressed() {
    let events = run_with_filter_and_capture();

    let warn_passed = events.iter().any(|e| {
        e.target == "chromiumoxide::handler"
            && e.level == tracing::Level::WARN
            && e.message.contains("ResetWithoutClosingHandshake")
    });

    assert!(
        warn_passed,
        "non-ERROR chromiumoxide events stay visible; only ERROR is suppressed"
    );
}
