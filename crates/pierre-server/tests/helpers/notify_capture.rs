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
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::{set_default, DefaultGuard};
use tracing::{Event, Metadata, Subscriber};

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

/// A whole `Subscriber`, not a `Layer` over a `Registry`.
///
/// The distinction is load-bearing. `common::init_test_logging` installs a
/// process-global `fmt` subscriber over its own `Registry`; layering this
/// capture over a SECOND `Registry` as the thread-local default put two
/// registries in one process with disjoint span-id spaces. sqlx-sqlite ships a
/// `tracing::Span` to its worker thread with every command, and when the span
/// is dropped there, tracing-subscriber closes its parent through whatever
/// dispatcher is current on the DROPPING thread — the global registry, which
/// has never seen that id — and panics `tried to drop a ref to Id(..), but no
/// such span exists!`. That killed the sqlx worker and, behind a one-connection
/// test pool, stalled the next acquire into a 500.
///
/// Capture never needed span storage: it reads events only. Owning no spans
/// means no id can be minted here for another registry to choke on, and a
/// foreign span closed on this thread reaches a `try_close` that does nothing.
impl Subscriber for NotifyCapture {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    // One id for every span: nothing here looks a span up, and the id is never
    // handed to a registry that would try to resolve it.
    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}

    fn event(&self, event: &Event<'_>) {
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
    let guard = set_default(capture);
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
