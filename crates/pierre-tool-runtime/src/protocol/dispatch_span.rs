// ABOUTME: The span every tool execution runs in: tool, protocol, user, tenant and outcome
// ABOUTME: Built and closed by UniversalExecutor::execute_tool, the chokepoint every transport funnels through
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! One span per tool execution.
//!
//! `UniversalExecutor::execute_tool` is the single dispatch path for the chat
//! tool loop, MCP-direct, A2A and an ACP agent's tool-host session, so a span
//! opened there covers every tool call exactly once. Its duration is the
//! span's own; the fields say whose call it was and how it ended.
//!
//! Only identifiers and machine codes go on the span. The arguments, the
//! result and an error's message stay off it: they carry the athlete's data
//! and, for `InternalError`, free text a tool body wrote.

use serde_json::Value as JsonValue;
use tracing::field::Empty;
use tracing::{info_span, Span};

use super::types::{UniversalRequest, UniversalResponse};
use crate::conversions::RAISED_ERROR_CODE_KEY;
use crate::protocols::ProtocolError;

/// Key an in-band refusal (Guardian, tenant tool-disable) names its code under.
const IN_BAND_ERROR_CODE_KEY: &str = "error_code";

/// The span `request`'s execution runs in. `outcome` and `error_code` are
/// filled by [`record_outcome`] once the call answers.
pub(super) fn tool_call_span(request: &UniversalRequest) -> Span {
    let span = info_span!(
        "execute_tool",
        tool_name = %request.tool_name,
        protocol = %request.protocol,
        user_id = %request.user_id,
        tenant_id = Empty,
        outcome = Empty,
        error_code = Empty,
    );
    if let Some(tenant_id) = request.tenant_id.as_deref() {
        span.record("tenant_id", tenant_id);
    }
    span
}

/// Record how the call ended on its span: `success`, `error` for a call the
/// tool answered in-band with a failure, or `rejected` for one the executor
/// refused with a [`ProtocolError`]. `error_code` names the failure by code,
/// never by message.
pub(super) fn record_outcome(span: &Span, result: &Result<UniversalResponse, ProtocolError>) {
    match result {
        Ok(response) if response.success => {
            span.record("outcome", "success");
        }
        Ok(response) => {
            span.record("outcome", "error");
            if let Some(code) = in_band_error_code(response) {
                span.record("error_code", code);
            }
        }
        Err(error) => {
            span.record("outcome", "rejected");
            span.record("error_code", protocol_error_code(error));
        }
    }
}

/// The machine code an in-band failure carries in its payload, if any.
fn in_band_error_code(response: &UniversalResponse) -> Option<&str> {
    let result = response.result.as_ref()?;
    [IN_BAND_ERROR_CODE_KEY, RAISED_ERROR_CODE_KEY]
        .into_iter()
        .find_map(|key| result.get(key).and_then(JsonValue::as_str))
}

/// A stable code per [`ProtocolError`] variant. Exhaustive on purpose: a new
/// variant must be named here, not folded into a catch-all.
const fn protocol_error_code(error: &ProtocolError) -> &'static str {
    match error {
        ProtocolError::UnsupportedProtocol { .. } => "unsupported_protocol",
        ProtocolError::ToolNotFound { .. } => "tool_not_found",
        ProtocolError::InvalidParameter { .. } => "invalid_parameter",
        ProtocolError::MissingParameter { .. } => "missing_parameter",
        ProtocolError::ConfigMissing { .. } => "config_missing",
        ProtocolError::ConversionFailed { .. } => "conversion_failed",
        ProtocolError::Serialization { .. }
        | ProtocolError::SerializationErrorDetailed { .. }
        | ProtocolError::SerializationError(_) => "serialization",
        ProtocolError::Database { .. } => "database",
        ProtocolError::InvalidSchema { .. } => "invalid_schema",
        ProtocolError::PermissionDenied { .. } => "permission_denied",
        ProtocolError::RateLimitExceeded { .. } => "rate_limit_exceeded",
        ProtocolError::InvalidRequestDetailed { .. } | ProtocolError::InvalidRequest(_) => {
            "invalid_request"
        }
        ProtocolError::ConfigurationErrorDetailed { .. } | ProtocolError::ConfigurationError(_) => {
            "configuration"
        }
        ProtocolError::InvalidParameters(_) => "invalid_parameters",
        ProtocolError::ExecutionFailedDetailed { .. } | ProtocolError::ExecutionFailed(_) => {
            "execution_failed"
        }
        ProtocolError::InternalError(_) => "internal",
        ProtocolError::ProviderAuthRequired { .. } => "provider_auth_required",
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
    use tracing_subscriber::registry::{self, LookupSpan};

    /// Captures, for every event, the merged fields of its ancestor spans —
    /// what the GCP formatter stamps onto a log line.
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<Map<String, JsonValue>>>>);

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

    fn request(tenant_id: Option<&str>) -> UniversalRequest {
        UniversalRequest {
            tool_name: "get_activities".to_owned(),
            parameters: json!({ "limit": 5 }),
            user_id: "11111111-1111-1111-1111-111111111111".to_owned(),
            protocol: "mcp".to_owned(),
            tenant_id: tenant_id.map(ToOwned::to_owned),
        }
    }

    fn response(success: bool, result: Option<JsonValue>) -> UniversalResponse {
        UniversalResponse {
            success,
            result,
            error: (!success).then(|| "refused".to_owned()),
            metadata: None,
        }
    }

    /// The fields a log line inside the span carries, after the call answered.
    fn fields_after(
        request: &UniversalRequest,
        result: &Result<UniversalResponse, ProtocolError>,
    ) -> Map<String, JsonValue> {
        let capture = Capture::default();
        let subscriber = registry::Registry::default()
            .with(SpanFieldStorage)
            .with(capture.clone());
        with_default(subscriber, || {
            let span = tool_call_span(request);
            record_outcome(&span, result);
            let _entered = span.enter();
            info!("inside the tool call");
        });
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
    fn span_carries_tool_user_tenant_and_success() {
        let tenant = "22222222-2222-2222-2222-222222222222";
        let fields = fields_after(&request(Some(tenant)), &Ok(response(true, None)));

        assert_eq!(
            fields.get("tool_name"),
            Some(&json!("get_activities")),
            "bug: tool name not on the span"
        );
        assert_eq!(
            fields.get("protocol"),
            Some(&json!("mcp")),
            "bug: protocol not on the span"
        );
        assert_eq!(
            fields.get("user_id"),
            Some(&json!("11111111-1111-1111-1111-111111111111")),
            "bug: user not on the span"
        );
        assert_eq!(
            fields.get("tenant_id"),
            Some(&json!(tenant)),
            "bug: tenant not on the span"
        );
        assert_eq!(
            fields.get("outcome"),
            Some(&json!("success")),
            "bug: a successful call is a success"
        );
        assert_eq!(
            fields.get("error_code"),
            None,
            "bug: a success carries no error code"
        );
    }

    #[test]
    fn span_never_carries_the_arguments() {
        let fields = fields_after(&request(None), &Ok(response(true, None)));
        assert!(
            fields.get("parameters").is_none(),
            "bug: the arguments must never reach the span"
        );
        assert!(
            fields.values().all(|v| v != &json!({ "limit": 5 })),
            "bug: the argument values must never reach the span"
        );
        assert_eq!(
            fields.get("tenant_id"),
            None,
            "bug: a tenant-less call names no tenant"
        );
    }

    #[test]
    fn in_band_failure_records_its_code() {
        let result = Ok(response(
            false,
            Some(json!({ "error_code": "guardian_denied", "reason": "taint" })),
        ));
        let fields = fields_after(&request(Some("t")), &result);
        assert_eq!(
            fields.get("outcome"),
            Some(&json!("error")),
            "bug: an in-band failure is an error"
        );
        assert_eq!(
            fields.get("error_code"),
            Some(&json!("guardian_denied")),
            "bug: the in-band code must be recorded"
        );
    }

    #[test]
    fn raised_failure_records_its_code() {
        let result = Ok(response(
            false,
            Some(json!({ RAISED_ERROR_CODE_KEY: "PERMISSION_DENIED" })),
        ));
        let fields = fields_after(&request(Some("t")), &result);
        assert_eq!(
            fields.get("error_code"),
            Some(&json!("PERMISSION_DENIED")),
            "bug: a raised error's code must be recorded"
        );
    }

    #[test]
    fn protocol_error_records_the_variant_code_not_the_message() {
        let result = Err(ProtocolError::InternalError(
            "athlete wrote: secret text".to_owned(),
        ));
        let fields = fields_after(&request(Some("t")), &result);
        assert_eq!(
            fields.get("outcome"),
            Some(&json!("rejected")),
            "bug: a ProtocolError is a rejection"
        );
        assert_eq!(
            fields.get("error_code"),
            Some(&json!("internal")),
            "bug: the variant code must be recorded"
        );
        assert!(
            fields
                .values()
                .all(|v| !v.as_str().is_some_and(|s| s.contains("secret"))),
            "bug: the error message must never reach the span"
        );
    }
}
