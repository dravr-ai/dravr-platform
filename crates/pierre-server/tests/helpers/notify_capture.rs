// ABOUTME: The suites' view of `target: "notify"` events — the product-analytics chokepoint — or of every log line
// ABOUTME: A thin reading of pierre_test_support::log_capture, which installs and collects every capture
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Shared helpers compile into every integration-test binary; the ones that
// do not assert on notify events see this module as dead.
#![allow(dead_code)]
#![allow(clippy::unwrap_used, clippy::panic)]

use std::collections::HashMap;

use pierre_test_support::log_capture::{self, CapturedEvent, CapturedLog};
use tracing::subscriber::DefaultGuard;
use tracing::Level;

/// One `target: "notify"` event, with every field rendered as a string.
///
/// Under [`capture_logs`] it is any log line, and `event` is its message.
#[derive(Clone, Debug)]
pub(crate) struct NotifyEvent {
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
    pub(crate) fn field(&self, name: &str) -> &str {
        self.fields.get(name).map_or_else(
            || panic!("event {} has no field {name}", self.event),
            String::as_str,
        )
    }
}

/// What a capture installed by this module has seen: the `notify` events, or
/// under [`capture_logs`] every line, each named.
#[derive(Clone, Debug)]
pub(crate) struct CapturedEvents {
    log: CapturedLog,
    every_line: bool,
}

impl CapturedEvents {
    /// The events captured so far, in the order emitted.
    pub(crate) fn events(&self) -> Vec<NotifyEvent> {
        self.log
            .events()
            .into_iter()
            .filter_map(|event| self.named_view(event))
            .collect()
    }

    fn named_view(&self, event: CapturedEvent) -> Option<NotifyEvent> {
        if !self.every_line && event.target != "notify" {
            return None;
        }
        let name = if self.every_line {
            event.message().unwrap_or_default().to_owned()
        } else {
            event
                .field("event")
                .unwrap_or_else(|| panic!("notify event with no `event` field: {event:?}"))
                .to_owned()
        };
        Some(NotifyEvent {
            event: name,
            level: event.level,
            fields: event.fields,
        })
    }
}

/// Capture this thread's `notify` events until the guard drops.
///
/// The guard must stay alive for the duration of the code under test. Only
/// code running on this thread is seen, which is every handler and route a
/// `#[tokio::test]` drives inline.
pub(crate) fn capture_notify() -> (CapturedEvents, DefaultGuard) {
    let (log, guard) = log_capture::capture_events();
    (
        CapturedEvents {
            log,
            every_line: false,
        },
        guard,
    )
}

/// Capture every log line this thread writes, at any level and target, named
/// by its message.
///
/// For asserting a structured log line that is itself the product — an
/// observe-mode report — rather than a notify event. The same guard rules as
/// [`capture_notify`] apply.
pub(crate) fn capture_logs() -> (CapturedEvents, DefaultGuard) {
    let (log, guard) = log_capture::capture_events();
    (
        CapturedEvents {
            log,
            every_line: true,
        },
        guard,
    )
}

/// [`capture_logs`], plus every value recorded onto a span after its
/// creation (read with [`CapturedLog::recorded`]): the fields a handler fills
/// in as it learns them, such as the token endpoint's `app_attest`.
pub(crate) fn capture_logs_and_spans() -> (CapturedEvents, CapturedLog, DefaultGuard) {
    let (log, guard) = log_capture::capture_events_and_spans();
    let events = CapturedEvents {
        log: log.clone(),
        every_line: true,
    };
    (events, log, guard)
}

/// Every captured event with this name.
pub(crate) fn named(events: &CapturedEvents, name: &str) -> Vec<NotifyEvent> {
    events
        .events()
        .into_iter()
        .filter(|e| e.event == name)
        .collect()
}

/// Exactly one event with this name, or a panic naming what was seen instead.
pub(crate) fn only(events: &CapturedEvents, name: &str) -> NotifyEvent {
    let matching = named(events, name);
    assert_eq!(
        matching.len(),
        1,
        "expected exactly one `{name}`, saw {:?}",
        events
            .events()
            .iter()
            .map(|e| e.event.clone())
            .collect::<Vec<_>>()
    );
    matching.into_iter().next().unwrap()
}
