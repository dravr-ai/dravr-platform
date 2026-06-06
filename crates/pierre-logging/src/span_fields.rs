// ABOUTME: tracing Layer capturing each span's field values into structured storage
// ABOUTME: so downstream formatters can propagate ancestor-span context onto child events
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Span-field storage layer.
//!
//! `tracing`'s registry keeps span field *values* only as the per-formatter
//! `FormattedFields` string; the structured key/value pairs are not retained in
//! a form a custom event formatter can read back. This layer records each
//! span's fields into a [`SpanFields`] map stored in the span's extensions, so
//! [`crate::gcp::GcpFormatter`] can copy turn-scoped context
//! (`user_id`, `tenant_id`, `conversation_id`, `turn_id`, `coach_id`,
//! `group_id`, …) onto every child event — without each call site re-passing
//! those identifiers.
//!
//! Fields declared `Empty` in an `#[instrument]` attribute are not recorded
//! until a later `Span::record(...)` call; [`SpanFieldStorage::on_record`]
//! folds those late values into the same map, so deferred identifiers (the
//! conversation's `coach_id`/`group_id`, resolved mid-turn) still propagate.

use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

use std::error::Error as StdError;
use std::fmt;

/// Structured snapshot of a span's recorded fields, stored in the span's
/// extensions by [`SpanFieldStorage`].
#[derive(Debug, Default, Clone)]
pub struct SpanFields(pub Map<String, Value>);

/// Layer that records span field values into [`SpanFields`] on span creation
/// and merges later `Span::record(...)` updates into the same map.
pub struct SpanFieldStorage;

impl<S> Layer<S> for SpanFieldStorage
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else {
            return;
        };
        let mut fields = SpanFields::default();
        attrs.record(&mut SpanFieldVisitor(&mut fields.0));
        span.extensions_mut().insert(fields);
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else {
            return;
        };
        let mut extensions = span.extensions_mut();
        if let Some(fields) = extensions.get_mut::<SpanFields>() {
            values.record(&mut SpanFieldVisitor(&mut fields.0));
        } else {
            let mut fields = SpanFields::default();
            values.record(&mut SpanFieldVisitor(&mut fields.0));
            extensions.insert(fields);
        }
    }
}

/// Visitor folding span field values into a `serde_json` map. Mirrors the
/// event visitor in [`crate::gcp`] but keeps `message` as an ordinary field —
/// a span's `message` (rare) is context, not the event line.
struct SpanFieldVisitor<'a>(&'a mut Map<String, Value>);

impl Visit for SpanFieldVisitor<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.0
            .insert(field.name().to_owned(), Value::String(format!("{value:?}")));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0
            .insert(field.name().to_owned(), Value::String(value.to_owned()));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.insert(field.name().to_owned(), Value::Bool(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0
            .insert(field.name().to_owned(), Value::Number(value.into()));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0
            .insert(field.name().to_owned(), Value::Number(value.into()));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        let rendered = serde_json::Number::from_f64(value)
            .map_or_else(|| Value::String(value.to_string()), Value::Number);
        self.0.insert(field.name().to_owned(), rendered);
    }

    fn record_error(&mut self, field: &Field, value: &(dyn StdError + 'static)) {
        self.0
            .insert(field.name().to_owned(), Value::String(value.to_string()));
    }
}
