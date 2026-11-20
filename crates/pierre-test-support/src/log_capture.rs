// ABOUTME: Test-only capture of one tracing target's events, for asserting the lines code logs
// ABOUTME: Shared by the quota ladder's notify tests and the usage-limit log tests (carnet#1630)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::fmt::Debug as FmtDebug;
use std::mem;
use std::sync::{Arc, Mutex, PoisonError};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::{self, DefaultGuard};
use tracing::{Event, Metadata, Subscriber};

/// The fields of every event on the captured target emitted on this thread
/// while the capture is installed, each rendered as text.
pub type CapturedEvents = Arc<Mutex<Vec<HashMap<String, String>>>>;

#[derive(Default)]
struct FieldText(HashMap<String, String>);

impl Visit for FieldText {
    fn record_debug(&mut self, field: &Field, value: &dyn FmtDebug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.insert(field.name().to_owned(), value.to_string());
    }
}

/// A whole subscriber that keeps one target's events and owns no spans,
/// so no span id it hands out is ever resolved elsewhere.
struct TargetCapture(CapturedEvents, &'static str);

impl Subscriber for TargetCapture {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        if event.metadata().target() != self.1 {
            return;
        }
        let mut fields = FieldText::default();
        event.record(&mut fields);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(fields.0);
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

/// Capture this thread's `notify` events until the guard drops.
#[must_use]
pub fn capture_notify() -> (CapturedEvents, DefaultGuard) {
    capture_target("notify")
}

/// Capture this thread's events on `target` until the guard drops.
#[must_use]
pub fn capture_target(target: &'static str) -> (CapturedEvents, DefaultGuard) {
    let capture = TargetCapture(CapturedEvents::default(), target);
    let events = Arc::clone(&capture.0);
    (events, subscriber::set_default(capture))
}

/// The captured events, leaving none behind.
#[must_use]
pub fn drain(events: &CapturedEvents) -> Vec<HashMap<String, String>> {
    mem::take(&mut *events.lock().unwrap_or_else(PoisonError::into_inner))
}
