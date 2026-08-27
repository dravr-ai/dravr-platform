// ABOUTME: Test-side capture of `target: "notify"` events — the product-analytics chokepoint
// ABOUTME: Installed per thread so a test asserts which events fired, how often, and with which fields
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Shared helpers compile into every integration-test binary; the ones that
// do not assert on notify events see this module as dead.
#![allow(dead_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::fmt::Debug as FmtDebug;
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::subscriber::DefaultGuard;
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

/// One `target: "notify"` event, with every field rendered as a string.
#[derive(Clone, Debug)]
pub struct NotifyEvent {
    /// The catalogued event name (`coach.installed`, `group.created`, …).
    pub event: String,
    /// Every field the emission carried, rendered as text.
    pub fields: HashMap<String, String>,
}

impl NotifyEvent {
    /// A field's rendered value; panics naming the event when it is absent.
    pub fn field(&self, name: &str) -> &str {
        self.fields
            .get(name)
            .unwrap_or_else(|| panic!("event {} has no field {name}", self.event))
    }
}

/// Every event captured since [`capture_notify`] installed the layer.
pub type CapturedEvents = Arc<Mutex<Vec<NotifyEvent>>>;

#[derive(Clone, Default)]
struct NotifyCapture {
    events: CapturedEvents,
}

#[derive(Debug, Default)]
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

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields
            .insert(field.name().to_owned(), value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields
            .insert(field.name().to_owned(), value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields
            .insert(field.name().to_owned(), value.to_string());
    }
}

impl<S> Layer<S> for NotifyCapture
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if event.metadata().target() != "notify" {
            return;
        }
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        let name = visitor
            .fields
            .get("event")
            .cloned()
            .unwrap_or_else(|| panic!("notify event with no `event` field: {visitor:?}"));
        self.events.lock().unwrap().push(NotifyEvent {
            event: name,
            fields: visitor.fields,
        });
    }
}

/// Install a capture subscriber for the current thread.
///
/// The guard must stay alive for the duration of the code under test — the
/// subscriber is uninstalled when it drops. Only code running on this thread
/// is seen, which is every handler and route a `#[tokio::test]` drives inline.
pub fn capture_notify() -> (CapturedEvents, DefaultGuard) {
    let capture = NotifyCapture::default();
    let events = Arc::clone(&capture.events);
    let guard = tracing_subscriber::registry().with(capture).set_default();
    (events, guard)
}

/// Every captured event with this name.
pub fn named(events: &CapturedEvents, name: &str) -> Vec<NotifyEvent> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| e.event == name)
        .cloned()
        .collect()
}

/// Exactly one event with this name, or a panic naming what was seen instead.
pub fn only(events: &CapturedEvents, name: &str) -> NotifyEvent {
    let matching = named(events, name);
    assert_eq!(
        matching.len(),
        1,
        "expected exactly one `{name}`, saw {:?}",
        events
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.event.clone())
            .collect::<Vec<_>>()
    );
    matching.into_iter().next().unwrap()
}
