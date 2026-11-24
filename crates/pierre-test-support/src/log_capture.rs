// ABOUTME: The one way a test installs a tracing subscriber and captures the lines code logs
// ABOUTME: Keeps two inert dispatchers registered so no sibling test can cache a shared log site as never
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Capture what code under test logs, on the test's own thread.
//!
//! Every capture goes through [`install`], which keeps two inert
//! dispatchers registered for the life of the test binary before it installs
//! the test's scoped subscriber. Without them a capture can lose a line.
//!
//! ## Why a capture loses lines
//!
//! `tracing` caches, per log site, whether any subscriber is interested in it.
//! In `tracing-core` 0.1.36, while at most one dispatcher is registered
//! (`has_just_one`), a log site registering for the first time computes that
//! interest from the dispatcher of the thread it registers on, without taking
//! the dispatcher lock (`Rebuilder::JustOne`). A sibling test reaching a
//! shared log site first, on a thread with no subscriber, caches it as
//! "never" for every thread, and the test that holds a capture sees nothing.
//! A `callsite::rebuild_interest_cache()` after installing does not close it:
//! a sibling registering the site after the rebuild still caches "never".
//!
//! The count is stored when a dispatcher registers, so it takes two
//! registered at once to turn that path off. The inert pair is never dropped,
//! so from then on every log site registers under the dispatcher lock, from
//! every live dispatcher, the test's capture included, and installing a
//! capture rebuilds every cached interest under that same lock.
//!
//! ## What remains
//!
//! One window `tracing` gives no way to close: a sibling thread that read
//! `has_just_one` before the inert pair registered, and is still between that
//! read and storing its "never" when a capture is installed. [`prime`] makes
//! the pair the first thing the binary's first capture or test database does,
//! then pauses once for `SETTLE` so such a registration finishes before any
//! capture installs; the rebuild that installing does overwrites what it
//! stored. A sibling descheduled across that whole pause, mid-registration of
//! the very log site a capture asserts on, can still store "never" after the
//! rebuild, until the next capture installs. Registering the pair before the
//! test harness starts would close it, but that needs a pre-`main`
//! constructor, which is `unsafe` code this workspace denies.
//!
//! ## `log` records
//!
//! [`prime`] also installs the `log` bridge (`tracing_log::LogTracer`), once
//! per binary, so a record a dependency writes through the `log` crate reaches
//! every capture as an event, whatever order the binary's tests run in.
//! `tracing_subscriber`'s `try_init` sets its global subscriber before it
//! tries the bridge, so one that runs after [`prime`] still installs its
//! subscriber and then returns the bridge's "already set" error, which a
//! caller that tolerates a set global (`.ok()`) ignores; its panicking `init`
//! would not. When that `try_init` ran first, its bridge is the one in place
//! and caps `log` records at its subscriber's max level, so a capture sees the
//! records at or above it (an `error!` always).
//!
//! ## Thread scope
//!
//! A capture is a thread's scoped default. It sees what runs on the
//! installing thread, which is everything a `#[tokio::test]` drives inline on
//! its current-thread runtime, and nothing a task spawned onto another thread
//! or `spawn_blocking` logs.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug as FmtDebug;
use std::mem;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError};
use std::thread::{self, ThreadId};
use std::time::Duration;

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::{self, DefaultGuard, NoSubscriber};
use tracing::{Dispatch, Event, Level, Metadata, Subscriber};
use tracing_core::span::Current;
use tracing_log::LogTracer;

/// The fields of every event on the captured target emitted on this thread
/// while the capture is installed, each rendered as text.
pub type CapturedEvents = Arc<Mutex<Vec<HashMap<String, String>>>>;

/// How long the binary's first [`prime`] waits after registering the inert
/// pair, for a sibling that began registering a log site before them.
const SETTLE: Duration = Duration::from_millis(50);

/// The inert pair, registered once and never dropped, and the `log` bridge.
static INERT_DISPATCHERS: LazyLock<[Dispatch; 2]> = LazyLock::new(|| {
    // An earlier global `try_init` may have installed the bridge already;
    // either way it is in place from here on.
    LogTracer::init().ok();
    let inert = [
        Dispatch::new(NoSubscriber::default()),
        Dispatch::new(NoSubscriber::default()),
    ];
    thread::sleep(SETTLE);
    inert
});

/// Register the inert pair, and the `log` bridge, for the rest of the test
/// binary, if not yet done.
///
/// Every capture calls it, and so does the test-database factory, so a test
/// that opens a database primes before its setup runs. Call it at the top of
/// any other test that captures after a long setup, so the pair registers
/// before the setup's log sites do.
pub fn prime() {
    LazyLock::force(&INERT_DISPATCHERS);
}

/// Install `subscriber` as this thread's default until the guard drops,
/// after [`prime`].
///
/// For a test whose subscriber is its own, such as a capture that keeps span
/// data. A test that only reads events uses [`capture_events`] or
/// [`capture_target`].
#[must_use]
pub fn install<S>(subscriber: S) -> DefaultGuard
where
    S: Subscriber + Send + Sync + 'static,
{
    prime();
    subscriber::set_default(subscriber)
}

/// One event logged while a capture was installed.
#[derive(Clone, Debug)]
pub struct CapturedEvent {
    /// The event's target.
    pub target: String,
    /// The level it was logged at.
    pub level: Level,
    /// Every field it carried, `message` included, rendered as text: a
    /// string as itself, anything else as its `Debug` form.
    pub fields: HashMap<String, String>,
}

impl CapturedEvent {
    /// The rendered value of `name`, when the event carried it.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }

    /// The event's message, when it had one.
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        self.field("message")
    }
}

/// Every event a [`capture_events`] capture has seen, in the order logged,
/// every value written onto a span while it was installed, and, under
/// [`capture_events_and_spans`], each span with its name and fields.
#[derive(Clone, Debug, Default)]
pub struct CapturedLog {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
    span_values: Arc<Mutex<Vec<String>>>,
    recorded: Arc<Mutex<Vec<(String, String)>>>,
    /// Each span [`capture_events_and_spans`] minted an id for, by that id;
    /// ids are minted in increasing order, so the map's order is the order
    /// opened.
    spans: Arc<Mutex<BTreeMap<u64, CapturedSpan>>>,
}

/// One span opened under [`capture_events_and_spans`]: its name and every
/// field it was opened or later recorded with, rendered as an event field is.
#[derive(Clone, Debug)]
pub struct CapturedSpan {
    /// The span's name.
    pub name: String,
    /// Its fields, a later record replacing the value it was opened with.
    pub fields: HashMap<String, String>,
}

impl CapturedLog {
    /// The events captured so far, leaving them in place.
    #[must_use]
    pub fn events(&self) -> Vec<CapturedEvent> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The events captured so far, leaving none behind.
    #[must_use]
    pub fn drain(&self) -> Vec<CapturedEvent> {
        mem::take(&mut *self.events.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Every value written onto any span, at its creation or later, rendered
    /// as an event field is, under either capture. A value here is not tied
    /// to its span ([`CapturedLog::spans`] keeps that, under
    /// [`capture_events_and_spans`] only): this is what a check that nothing
    /// reaches the log (an event or a field an event would carry from its
    /// span) scans.
    #[must_use]
    pub fn span_values(&self) -> Vec<String> {
        self.span_values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Every value recorded onto a span under `field` after its creation
    /// (`Span::record`), in the order recorded. Under [`capture_events`] only
    /// a span the code holds reaches the capture; [`capture_events_and_spans`]
    /// also sees `Span::current().record(..)`.
    #[must_use]
    pub fn recorded(&self, field: &str) -> Vec<String> {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(name, _)| name == field)
            .map(|(_, value)| value.clone())
            .collect()
    }

    /// Every `(field, value)` recorded onto a span after its creation, in the
    /// order recorded; [`CapturedLog::recorded`] reads one field of it.
    #[must_use]
    pub fn recorded_fields(&self) -> Vec<(String, String)> {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Every span opened under [`capture_events_and_spans`], in the order
    /// opened. Under [`capture_events`] no span is told apart, so none is kept.
    #[must_use]
    pub fn spans(&self) -> Vec<CapturedSpan> {
        self.spans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }

    fn keep_span(&self, id: u64, span: CapturedSpan) {
        self.spans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, span);
    }

    fn extend_span(&self, id: u64, fields: &FieldText) {
        if let Some(span) = self
            .spans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(&id)
        {
            span.fields.extend(
                fields
                    .0
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone())),
            );
        }
    }

    fn keep_recorded(&self, fields: &FieldText) {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(
                fields
                    .0
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone())),
            );
    }

    fn keep_span_values(&self, fields: FieldText) {
        self.span_values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(fields.0.into_values());
    }
}

#[derive(Default)]
struct FieldText(HashMap<String, String>);

impl Visit for FieldText {
    fn record_debug(&mut self, field: &Field, value: &dyn FmtDebug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
}

/// Where a [`Collector`] keeps what it saw.
enum Sink {
    /// Every event, with its target and level.
    Events(CapturedLog),
    /// The fields of each event on one target.
    Fields(CapturedEvents, &'static str),
}

/// Which span is current on each thread, for [`capture_events_and_spans`].
///
/// `Span::record` on `Span::current()` reaches a subscriber only when it can
/// say which span is current, so this mode mints an id per span and keeps the
/// entered stack. It still holds no span data a drop could fail to find.
#[derive(Default)]
struct SpanTracking {
    /// The last id minted; ids start above the shared id `1`.
    last_id: u64,
    metadata: HashMap<u64, &'static Metadata<'static>>,
    /// Entered spans with the thread that entered them: sqlx enters a span it
    /// was handed on its worker thread, and that span is never current on the
    /// test's thread.
    entered: Vec<(ThreadId, u64)>,
}

/// The subscriber every capture installs: it keeps events and owns no spans.
///
/// What it keeps of a span is a copy in the [`CapturedLog`], which a close
/// never looks up or releases, so a span created on the test thread and
/// dropped on another one (an sqlx `SQLite` worker does this) has nothing to
/// release. A `tracing_subscriber::Registry` in its place panics that worker
/// with "no such span exists" (0a002fe4a, 042b3a500).
struct Collector {
    sink: Sink,
    /// The current-span bookkeeping, only under [`capture_events_and_spans`].
    spans: Option<Mutex<SpanTracking>>,
}

impl Collector {
    const fn new(sink: Sink) -> Self {
        Self { sink, spans: None }
    }

    fn tracking(&self) -> Option<MutexGuard<'_, SpanTracking>> {
        self.spans
            .as_ref()
            .map(|spans| spans.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl Subscriber for Collector {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, span: &Attributes<'_>) -> Id {
        let mut fields = FieldText::default();
        span.record(&mut fields);
        let Some(mut tracking) = self.tracking() else {
            if let Sink::Events(log) = &self.sink {
                log.keep_span_values(fields);
            }
            return Id::from_u64(1);
        };
        tracking.last_id = tracking.last_id.max(1) + 1;
        let id = tracking.last_id;
        tracking.metadata.insert(id, span.metadata());
        drop(tracking);
        if let Sink::Events(log) = &self.sink {
            log.keep_span(
                id,
                CapturedSpan {
                    name: span.metadata().name().to_owned(),
                    fields: fields.0.clone(),
                },
            );
            log.keep_span_values(fields);
        }
        Id::from_u64(id)
    }

    fn record(&self, span: &Id, values: &Record<'_>) {
        if let Sink::Events(log) = &self.sink {
            let mut fields = FieldText::default();
            values.record(&mut fields);
            log.keep_recorded(&fields);
            log.extend_span(span.into_u64(), &fields);
            log.keep_span_values(fields);
        }
    }

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let metadata = event.metadata();
        if let Sink::Fields(_, target) = &self.sink {
            if metadata.target() != *target {
                return;
            }
        }
        let mut fields = FieldText::default();
        event.record(&mut fields);
        match &self.sink {
            Sink::Events(log) => {
                log.events
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(CapturedEvent {
                        target: metadata.target().to_owned(),
                        level: *metadata.level(),
                        fields: fields.0,
                    });
            }
            Sink::Fields(events, _) => {
                events
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(fields.0);
            }
        }
    }

    fn enter(&self, span: &Id) {
        if let Some(mut tracking) = self.tracking() {
            tracking
                .entered
                .push((thread::current().id(), span.into_u64()));
        }
    }

    fn exit(&self, span: &Id) {
        if let Some(mut tracking) = self.tracking() {
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
        // Without span tracking no span is ever current, as under the
        // trait's default answer.
        let Some(tracking) = self.tracking() else {
            return Current::none();
        };
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
}

/// Capture every event this thread logs, at any level and target, until the
/// guard drops.
#[must_use]
pub fn capture_events() -> (CapturedLog, DefaultGuard) {
    let log = CapturedLog::default();
    let guard = install(Collector::new(Sink::Events(log.clone())));
    (log, guard)
}

/// [`capture_events`], plus the spans opened and which one is current.
///
/// A value recorded onto `Span::current()` reaches [`CapturedLog::recorded`]:
/// the fields a handler fills in on its request span as it learns them. Each
/// span opened keeps its name and fields in [`CapturedLog::spans`].
#[must_use]
pub fn capture_events_and_spans() -> (CapturedLog, DefaultGuard) {
    let log = CapturedLog::default();
    let collector = Collector {
        sink: Sink::Events(log.clone()),
        spans: Some(Mutex::default()),
    };
    let guard = install(collector);
    (log, guard)
}

/// Run `run` under [`capture_events`], returning what it returned and every
/// event it logged.
pub fn capture_events_while<T>(run: impl FnOnce() -> T) -> (T, Vec<CapturedEvent>) {
    let (log, guard) = capture_events();
    let returned = run();
    drop(guard);
    (returned, log.drain())
}

/// Capture this thread's `notify` events until the guard drops.
#[must_use]
pub fn capture_notify() -> (CapturedEvents, DefaultGuard) {
    capture_target("notify")
}

/// Capture this thread's events on `target` until the guard drops.
#[must_use]
pub fn capture_target(target: &'static str) -> (CapturedEvents, DefaultGuard) {
    let events = CapturedEvents::default();
    let sink = Sink::Fields(Arc::clone(&events), target);
    let guard = install(Collector::new(sink));
    (events, guard)
}

/// The captured events, leaving none behind.
#[must_use]
pub fn drain(events: &CapturedEvents) -> Vec<HashMap<String, String>> {
    mem::take(&mut *events.lock().unwrap_or_else(PoisonError::into_inner))
}

#[cfg(test)]
mod tests {
    use tracing::field::Empty;
    use tracing::{info_span, Span};

    use super::*;

    /// A record written through the `log` crate reaches a capture as an
    /// event, so a check that nothing logged an error sees it too.
    #[test]
    fn a_log_crate_record_reaches_the_capture() {
        let ((), events) = capture_events_while(|| {
            log::error!(target: "log_capture_bridge", "written through the log crate");
        });
        let bridged: Vec<_> = events
            .iter()
            .filter(|event| event.message() == Some("written through the log crate"))
            .collect();
        assert_eq!(
            bridged.len(),
            1,
            "the log record was not bridged: {events:?}"
        );
        assert_eq!(
            bridged[0].level,
            Level::ERROR,
            "the bridged record lost its level: {bridged:?}"
        );
    }

    /// A value recorded onto the current span reaches the span capture, and
    /// only that one: the plain capture knows no current span.
    #[test]
    fn a_value_recorded_on_the_current_span_reaches_the_span_capture() {
        let (log, guard) = capture_events_and_spans();
        let span = info_span!("request", route = "/mcp", attested = Empty);
        {
            let _entered = span.enter();
            Span::current().record("attested", "yes");
        }
        drop(guard);
        let spans = log.spans();
        assert_eq!(
            spans.len(),
            1,
            "the span capture keeps the one span: {spans:?}"
        );
        assert_eq!(
            spans[0].name, "request",
            "the span capture lost the span's name: {spans:?}"
        );
        assert_eq!(
            spans[0].fields.get("route").map(String::as_str),
            Some("/mcp"),
            "a value the span was opened with stays on it: {spans:?}"
        );
        assert_eq!(
            spans[0].fields.get("attested").map(String::as_str),
            Some("yes"),
            "a value recorded later joins its span: {spans:?}"
        );
        assert_eq!(
            log.recorded("attested"),
            ["yes"],
            "the span capture did not see the value recorded on the current span"
        );

        let (plain, guard) = capture_events();
        let span = info_span!("request", attested = Empty);
        {
            let _entered = span.enter();
            Span::current().record("attested", "yes");
        }
        drop(guard);
        assert!(
            plain.recorded("attested").is_empty(),
            "the plain capture answered a current span it does not track"
        );
    }
}
