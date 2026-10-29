// ABOUTME: Records request identity (user, tenant) on every enclosing span that declares the field
// ABOUTME: so the HTTP root span, and every log and trace under it, carry who the request is for
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Identity recording across the span scope.
//!
//! `Span::record` writes to one span, and `tracing` silently drops a record on
//! a field that span did not declare. Authentication runs inside its own
//! `#[instrument]` span, which closes as soon as it returns, so an identity
//! recorded there never reaches the handler, LLM and database work that
//! follow. [`record_in_scope`] writes the value to every span in the current
//! scope that declares the field — the HTTP root span included — so the
//! identity outlives the auth span for every layer: the GCP formatter's
//! ancestor walk and the OpenTelemetry span attributes alike.

use std::fmt::Display;

use tracing::field::{display, Value};
use tracing::span::{Id, Record};
use tracing::{dispatcher, Metadata};
use tracing_subscriber::registry::{LookupSpan, Registry};

/// Span field naming the authenticated user.
pub const USER_ID_FIELD: &str = "user_id";
/// Span field naming the tenant the request acts in.
pub const TENANT_ID_FIELD: &str = "tenant_id";

/// Record `value` under `field` on the current span and every ancestor that
/// declares `field`.
///
/// Spans that do not declare the field are skipped, as `Span::record` would
/// skip them. Under a subscriber that is not built on a [`Registry`] the
/// ancestors cannot be looked up, so only the current span is written.
pub fn record_in_scope(field: &str, value: &dyn Value) {
    // Everything goes through `dispatch`: `Span::current()` re-enters the
    // dispatcher, and `tracing` answers a nested `get_default` with the no-op
    // subscriber, so a record made that way from in here is silently lost.
    dispatcher::get_default(|dispatch| {
        let current_span = dispatch.current_span();
        let (Some(current), Some(current_metadata)) =
            (current_span.id().cloned(), current_span.metadata())
        else {
            return;
        };
        // Collect first, record after: a layer's `on_record` takes the span's
        // extensions lock, which must not be contended by a live scope walk.
        let targets: Vec<(Id, &'static Metadata<'static>)> =
            match dispatch.downcast_ref::<Registry>() {
                Some(registry) => registry
                    .span(&current)
                    .into_iter()
                    .flat_map(|span| span.scope())
                    .filter(|span| span.metadata().fields().field(field).is_some())
                    .map(|span| (span.id(), span.metadata()))
                    .collect(),
                None => vec![(current, current_metadata)],
            };
        for (id, metadata) in targets {
            let fields = metadata.fields();
            if let Some(key) = fields.field(field) {
                let values = [(&key, Some(value))];
                dispatch.record(&id, &Record::new(&fields.value_set(&values)));
            }
        }
    });
}

/// Record the authenticated user and, when known, the tenant on every
/// enclosing span that declares `user_id` / `tenant_id`.
pub fn record_identity(user_id: impl Display, tenant_id: Option<impl Display>) {
    record_in_scope(USER_ID_FIELD, &display(user_id));
    if let Some(tenant_id) = tenant_id {
        record_in_scope(TENANT_ID_FIELD, &display(tenant_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Map, Value as Json};
    use std::sync::{Arc, Mutex, PoisonError};
    use tracing::field::Empty;
    use tracing::subscriber::with_default;
    use tracing::{info, info_span, Event, Subscriber};
    use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
    use tracing_subscriber::registry;

    use crate::span_fields::{SpanFieldStorage, SpanFields};

    /// Captures, for every event, the merged fields of its ancestor spans —
    /// what the GCP formatter stamps onto a log line.
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<Map<String, Json>>>>);

    impl<S> Layer<S> for Capture
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
            let mut merged = Map::new();
            if let Some(scope) = ctx.event_scope(event) {
                for span in scope.from_root() {
                    if let Some(fields) = span.extensions().get::<SpanFields>() {
                        merged.extend(fields.0.clone());
                    }
                }
            }
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(merged);
        }
    }

    #[test]
    fn identity_recorded_inside_auth_span_reaches_the_root_span() {
        let capture = Capture::default();
        let subscriber = registry().with(SpanFieldStorage).with(capture.clone());

        with_default(subscriber, || {
            let root = info_span!("request", user_id = Empty, tenant_id = Empty);
            let _root = root.enter();
            {
                // Auth's own span, closed before the handler runs.
                let auth = info_span!("authenticate", user_id = Empty, tenant_id = Empty);
                let _auth = auth.enter();
                record_identity("user-1", Some("tenant-1"));
            }
            // A handler span that does not declare the fields must not break
            // the walk, and its events still inherit from the root.
            let handler = info_span!("handler");
            let _handler = handler.enter();
            info!("after auth");
        });

        let events = capture
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        assert_eq!(events.len(), 1, "the handler logged exactly one line");
        assert_eq!(
            events[0].get("user_id"),
            Some(&Json::from("user-1")),
            "bug: the user recorded inside the auth span must reach the root span"
        );
        assert_eq!(
            events[0].get("tenant_id"),
            Some(&Json::from("tenant-1")),
            "bug: the tenant recorded inside the auth span must reach the root span"
        );
    }

    #[test]
    fn absent_tenant_leaves_the_field_empty() {
        let capture = Capture::default();
        let subscriber = registry().with(SpanFieldStorage).with(capture.clone());

        with_default(subscriber, || {
            let root = info_span!("request", user_id = Empty, tenant_id = Empty);
            let _root = root.enter();
            record_identity("user-2", None::<&str>);
            info!("no tenant");
        });

        let events = capture
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        assert_eq!(
            events[0].get("user_id"),
            Some(&Json::from("user-2")),
            "bug: the user must be recorded without a tenant"
        );
        assert_eq!(
            events[0].get("tenant_id"),
            None,
            "bug: an absent tenant must leave the field empty, not write a placeholder"
        );
    }

    #[test]
    fn no_current_span_is_a_no_op() {
        let subscriber = registry().with(SpanFieldStorage);
        with_default(subscriber, || {
            record_identity("user-3", Some("tenant-3"));
        });
    }
}
