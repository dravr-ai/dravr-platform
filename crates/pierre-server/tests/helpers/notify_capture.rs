// ABOUTME: Test-side capture of `target: "notify"` events — the product-analytics chokepoint — or of every log line
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
use std::thread::{self, ThreadId};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::{set_default, DefaultGuard};
use tracing::{Event, Level, Metadata, Subscriber};
use tracing_core::span::Current;

/// One `target: "notify"` event, with every field rendered as a string.
///
/// Under [`capture_logs`] it is any log line, and `event` is its message.
#[derive(Clone, Debug)]
pub struct NotifyEvent {
    /// The catalogued event name (`agent.installed`, `group.created`, …), or
    /// the message of a line [`capture_logs`] recorded.
    pub event: String,
    /// The level the line was emitted at.
    pub level: Level,
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

/// Every value recorded onto a span after it was created (`Span::record`),
/// as `(field, value)` in the order recorded, since
/// [`capture_logs_and_spans`] installed the subscriber.
///
/// The subscriber gives every span one id, so a value is not attributed to a
/// span: these are the values the code under test recorded, whichever span
/// was current.
pub type RecordedSpanFields = Arc<Mutex<Vec<(String, String)>>>;

#[derive(Clone, Default)]
struct NotifyCapture {
    events: CapturedEvents,
    /// Record every event under its message, not only `target: "notify"`.
    every_line: bool,
    /// Span tracking, only under [`capture_logs_and_spans`].
    spans: Option<SpanCapture>,
}

/// What [`capture_logs_and_spans`] keeps to see `Span::record` values.
///
/// `Span::record` on `Span::current()` reaches a subscriber only when the
/// subscriber can say which span is current, so this mode mints an id per
/// span and keeps the entered stack. The ids still never reach a registry:
/// every span carries the dispatcher that created it, and this one looks
/// nothing up in another.
#[derive(Clone, Default)]
struct SpanCapture {
    fields: RecordedSpanFields,
    tracking: Arc<Mutex<SpanTracking>>,
}

#[derive(Default)]
struct SpanTracking {
    /// The last id minted; ids start above the shared id `1`
    last_id: u64,
    metadata: HashMap<u64, &'static Metadata<'static>>,
    /// Entered spans with the thread that entered them: sqlx enters a span it
    /// was handed on its worker thread, and that span is never current on
    /// the test's thread
    entered: Vec<(ThreadId, u64)>,
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
    // handed to a registry that would try to resolve it. Span capture alone
    // mints its own ids, to know which span is current (see `SpanCapture`).
    fn new_span(&self, span: &Attributes<'_>) -> Id {
        let Some(spans) = &self.spans else {
            return Id::from_u64(1);
        };
        let mut tracking = spans.tracking.lock().unwrap();
        tracking.last_id = tracking.last_id.max(1) + 1;
        let id = tracking.last_id;
        tracking.metadata.insert(id, span.metadata());
        Id::from_u64(id)
    }

    fn record(&self, _span: &Id, values: &Record<'_>) {
        let Some(spans) = &self.spans else {
            return;
        };
        let mut visitor = FieldVisitor::default();
        values.record(&mut visitor);
        spans.fields.lock().unwrap().extend(visitor.fields);
    }

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn enter(&self, span: &Id) {
        if let Some(spans) = &self.spans {
            spans
                .tracking
                .lock()
                .unwrap()
                .entered
                .push((thread::current().id(), span.into_u64()));
        }
    }

    fn exit(&self, span: &Id) {
        if let Some(spans) = &self.spans {
            let mut tracking = spans.tracking.lock().unwrap();
            let entry = (thread::current().id(), span.into_u64());
            if let Some(at) = tracking
                .entered
                .iter()
                .rposition(|entered| *entered == entry)
            {
                tracking.entered.remove(at);
            }
        }
    }

    fn current_span(&self) -> Current {
        // Without span capture no span is ever current: `Span::current()`
        // is disabled, exactly as under the trait's default answer.
        let Some(spans) = &self.spans else {
            return Current::none();
        };
        let tracking = spans.tracking.lock().unwrap();
        let this_thread = thread::current().id();
        tracking
            .entered
            .iter()
            .rev()
            .find(|(thread, _)| *thread == this_thread)
            .and_then(|(_, id)| {
                tracking
                    .metadata
                    .get(id)
                    .map(|metadata| Current::new(Id::from_u64(*id), metadata))
            })
            .unwrap_or_else(Current::none)
    }

    fn event(&self, event: &Event<'_>) {
        if !self.every_line && event.metadata().target() != "notify" {
            return;
        }
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        let name = if self.every_line {
            visitor.fields.get("message").cloned().unwrap_or_default()
        } else {
            visitor
                .fields
                .get("event")
                .cloned()
                .unwrap_or_else(|| panic!("notify event with no `event` field: {visitor:?}"))
        };
        self.events.lock().unwrap().push(NotifyEvent {
            event: name,
            level: *event.metadata().level(),
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

/// Install a capture subscriber for the current thread that records every log
/// line, at any level and target, named by its message.
///
/// For asserting a structured log line that is itself the product — an
/// observe-mode report — rather than a notify event. The same guard rules as
/// [`capture_notify`] apply.
pub fn capture_logs() -> (CapturedEvents, DefaultGuard) {
    let capture = NotifyCapture {
        every_line: true,
        ..NotifyCapture::default()
    };
    let events = Arc::clone(&capture.events);
    let guard = set_default(capture);
    (events, guard)
}

/// [`capture_logs`], plus every value recorded onto a span after its
/// creation: the fields a handler fills in as it learns them, such as the
/// token endpoint's `app_attest`.
pub fn capture_logs_and_spans() -> (CapturedEvents, RecordedSpanFields, DefaultGuard) {
    let spans = SpanCapture::default();
    let span_fields = Arc::clone(&spans.fields);
    let capture = NotifyCapture {
        every_line: true,
        spans: Some(spans),
        ..NotifyCapture::default()
    };
    let events = Arc::clone(&capture.events);
    let guard = set_default(capture);
    (events, span_fields, guard)
}

/// Every value recorded onto a span under `field`, in the order recorded.
pub fn recorded(span_fields: &RecordedSpanFields, field: &str) -> Vec<String> {
    span_fields
        .lock()
        .unwrap()
        .iter()
        .filter(|(name, _)| name == field)
        .map(|(_, value)| value.clone())
        .collect()
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
