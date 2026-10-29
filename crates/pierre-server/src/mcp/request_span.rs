// ABOUTME: One span per MCP JSON-RPC operation: method, request id, target, caller and outcome
// ABOUTME: Installed on the tronc McpServer as its Observer; the host seams record the caller into it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The MCP request span.
//!
//! The tronc engine dispatches every JSON-RPC message, on every transport,
//! through its [`Observer`] hook, and runs the dispatch inside the span the
//! observer returns. [`RequestSpanObserver`] opens that span with what the
//! message says about itself, and records how it ended once it answers; its
//! duration is the span's own.
//!
//! Authentication runs before the observed dispatch, so the observer cannot
//! see the caller. The host seams receive the resolved [`ToolContext`] inside
//! the span and call [`record_caller`], which writes `user_id` / `tenant_id` on
//! every enclosing span that declares them — this one and the HTTP root alike.
//!
//! Payload capture stays at tronc's default (disabled): no tool arguments or
//! results reach this span, only identifiers and codes.

use std::time::Duration;

use dravr_tronc::mcp::observe::{Observer, OperationInfo, OperationOutcome};
use dravr_tronc::mcp::tool::ToolContext;
use pierre_logging::{record_in_scope, TENANT_ID_FIELD, USER_ID_FIELD};
use serde_json::Value;
use tracing::field::Empty;
use tracing::{info_span, Span};

/// Opens and closes the span each MCP operation runs in.
#[derive(Debug, Default)]
pub(crate) struct RequestSpanObserver;

impl Observer for RequestSpanObserver {
    fn start(&self, info: &OperationInfo) -> Span {
        request_span(
            &info.method,
            info.request_id.as_ref(),
            info.target.as_deref(),
        )
    }

    fn complete(
        &self,
        _info: &OperationInfo,
        span: &Span,
        outcome: &OperationOutcome,
        _elapsed: Duration,
    ) {
        record_outcome(span, outcome);
    }
}

/// The span one MCP operation runs in. `target` is the tool a `tools/call`
/// names or the prompt a `prompts/get` names.
fn request_span(method: &str, request_id: Option<&Value>, target: Option<&str>) -> Span {
    let span = info_span!(
        "mcp.request",
        method = %method,
        request_id = Empty,
        target = Empty,
        user_id = Empty,
        tenant_id = Empty,
        outcome = Empty,
        error_code = Empty,
    );
    if let Some(id) = request_id {
        // A string id as itself, a number as its digits.
        let rendered = id.as_str().map_or_else(|| id.to_string(), str::to_owned);
        span.record("request_id", rendered.as_str());
    }
    if let Some(target) = target {
        span.record("target", target);
    }
    span
}

/// Record how the operation ended. A JSON-RPC error records its numeric code;
/// its message stays off the span.
fn record_outcome(span: &Span, outcome: &OperationOutcome) {
    let label = match outcome {
        OperationOutcome::Completed { .. } => "success",
        OperationOutcome::ToolError { .. } => "tool_error",
        OperationOutcome::TaskCreated => "task_created",
        OperationOutcome::Failed { code, .. } => {
            span.record("error_code", i64::from(*code));
            "failed"
        }
        OperationOutcome::NotificationAccepted => "notification",
        OperationOutcome::Cancelled => "cancelled",
        // `OperationOutcome` is non-exhaustive: an outcome tronc adds later is
        // still recorded as ended rather than left blank.
        _ => "other",
    };
    span.record("outcome", label);
}

/// Record the authenticated caller on every enclosing span that declares
/// `user_id` / `tenant_id`: the MCP request span and the HTTP root span.
pub(crate) fn record_caller(ctx: &ToolContext) {
    if let Some(user_id) = ctx.user_id.as_deref() {
        record_in_scope(USER_ID_FIELD, &user_id);
    }
    if let Some(tenant_id) = ctx.tenant_id.as_deref() {
        record_in_scope(TENANT_ID_FIELD, &tenant_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_logging::span_fields::{SpanFieldStorage, SpanFields};
    use serde_json::{json, Map};
    use std::sync::{Arc, Mutex, PoisonError};
    use tracing::subscriber::with_default;
    use tracing::{info, Event, Subscriber};
    use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
    use tracing_subscriber::registry::{LookupSpan, Registry};

    /// Captures, for every event, the merged fields of its ancestor spans —
    /// what the GCP formatter stamps onto a log line.
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<Map<String, Value>>>>);

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

    /// Run `body` under a subscriber that stores span fields, and return the
    /// merged fields of the one event it emits.
    fn single_event(body: impl FnOnce()) -> Map<String, Value> {
        let capture = Capture::default();
        let subscriber = Registry::default()
            .with(SpanFieldStorage)
            .with(capture.clone());
        with_default(subscriber, body);
        let events = capture
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        assert_eq!(
            events.len(),
            1,
            "test misuse: the body must log exactly one line"
        );
        events.into_iter().next().unwrap_or_default()
    }

    #[test]
    fn caller_recorded_in_the_dispatch_reaches_the_request_span() {
        let ctx = ToolContext::default()
            .with_user("11111111-1111-1111-1111-111111111111".to_owned())
            .with_tenant("22222222-2222-2222-2222-222222222222".to_owned());

        let fields = single_event(|| {
            let span = request_span("tools/call", Some(&json!(7)), Some("get_activities"));
            span.in_scope(|| {
                // A seam's own span, closed before the operation answers.
                info_span!("call_tool").in_scope(|| record_caller(&ctx));
            });
            record_outcome(&span, &OperationOutcome::Completed { result: None });
            span.in_scope(|| info!("after dispatch"));
        });

        assert_eq!(
            fields.get("method"),
            Some(&json!("tools/call")),
            "bug: method not on the span"
        );
        assert_eq!(
            fields.get("request_id"),
            Some(&json!("7")),
            "bug: a numeric id renders as its digits"
        );
        assert_eq!(
            fields.get("target"),
            Some(&json!("get_activities")),
            "bug: the tool name is the target"
        );
        assert_eq!(
            fields.get("user_id"),
            Some(&json!("11111111-1111-1111-1111-111111111111")),
            "bug: the caller recorded in a seam span must reach the request span"
        );
        assert_eq!(
            fields.get("tenant_id"),
            Some(&json!("22222222-2222-2222-2222-222222222222")),
            "bug: the tenant recorded in a seam span must reach the request span"
        );
        assert_eq!(
            fields.get("outcome"),
            Some(&json!("success")),
            "bug: a completed call is a success"
        );
    }

    #[test]
    fn json_rpc_error_records_its_code_not_its_message() {
        let fields = single_event(|| {
            let span = request_span("tools/call", Some(&json!("abc")), None);
            record_outcome(
                &span,
                &OperationOutcome::Failed {
                    code: -32602,
                    message: "athlete wrote: secret text".to_owned(),
                },
            );
            span.in_scope(|| info!("after dispatch"));
        });

        assert_eq!(
            fields.get("request_id"),
            Some(&json!("abc")),
            "bug: a string id renders as itself"
        );
        assert_eq!(
            fields.get("outcome"),
            Some(&json!("failed")),
            "bug: a JSON-RPC error is a failure"
        );
        assert_eq!(
            fields.get("error_code"),
            Some(&json!(-32602)),
            "bug: the numeric code must be recorded"
        );
        assert!(
            fields
                .values()
                .all(|v| !v.as_str().is_some_and(|s| s.contains("secret"))),
            "bug: the error message must never reach the span"
        );
    }

    #[test]
    fn anonymous_caller_leaves_identity_empty() {
        let fields = single_event(|| {
            let span = request_span("initialize", Some(&json!(1)), None);
            span.in_scope(|| {
                record_caller(&ToolContext::default());
                info!("anonymous");
            });
        });

        assert_eq!(
            fields.get("user_id"),
            None,
            "bug: an anonymous call names no user"
        );
        assert_eq!(
            fields.get("tenant_id"),
            None,
            "bug: an anonymous call names no tenant"
        );
        assert_eq!(
            fields.get("method"),
            Some(&json!("initialize")),
            "bug: method not on the span"
        );
    }
}
