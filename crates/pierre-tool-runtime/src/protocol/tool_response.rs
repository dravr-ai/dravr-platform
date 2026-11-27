// ABOUTME: Reads a tool's ToolResponse back for the universal executor — raised error codes, sentinels, the reply
// ABOUTME: Rebuilds the ProtocolError a raised AppError maps to and converts the response into a UniversalResponse

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::mcp::schema::{Content, ToolResponse};
use pierre_core::errors::ErrorCode;
use serde_json::Value as JsonValue;

use crate::conversions::RAISED_ERROR_CODE_KEY;
use crate::protocol::types::UniversalResponse;
use crate::protocols::ProtocolError;

/// Detect the provider-auth sentinel a tool body encodes in `structured_content`
/// when it hit `ProviderAuthRequired`, returning the provider slug to re-raise.
pub(super) fn provider_auth_required_slug(response: &ToolResponse) -> Option<String> {
    let structured = response.structured_content.as_ref()?;
    if structured.get("error_code").and_then(|v| v.as_str()) == Some("provider_auth_required") {
        structured
            .get("provider")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned)
    } else {
        None
    }
}

/// Whether a raised error code is a read the provider's terms keep off the
/// transport the call was served over.
pub(super) fn is_unavailable_over_transport(code: &str) -> bool {
    matches!(
        serde_json::from_value(JsonValue::String(code.to_owned())),
        Ok(ErrorCode::UnavailableOverTransport)
    )
}

/// Read the `ErrorCode` tag a raised [`pierre_core::errors::AppError`] recorded
/// in `structured_content` (see [`RAISED_ERROR_CODE_KEY`]). Returns `None` for an
/// in-band `Ok(ToolResult::error(..))` failure, which carries no tag.
pub(super) fn raised_error_code(response: &ToolResponse) -> Option<String> {
    response
        .structured_content
        .as_ref()?
        .get(RAISED_ERROR_CODE_KEY)
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
}

/// Rebuild the [`ProtocolError`] a raised [`pierre_core::errors::AppError`] would
/// have produced before the E3 cutover, from the `ErrorCode` tag and message the
/// tool recorded in its [`ToolResponse`].
///
/// Mirrors the pre-E3 `AppError` → `ProtocolError` mapping: validation input maps
/// to `InvalidParameters`; auth / provider gating and not-found map to
/// `InvalidRequest`; an authorization refusal maps to `PermissionDenied`, which
/// carries the tool name itself; an upstream service's failure maps to
/// `ExecutionFailed`; every other code (database, internal, …) maps to
/// `InternalError`. The `tool_name` prefix and original message are preserved
/// so message-asserting callers keep matching.
pub(super) fn protocol_error_from_raised(
    tool_name: &str,
    error_code: &str,
    response: &ToolResponse,
) -> ProtocolError {
    let message = response
        .structured_content
        .as_ref()
        .and_then(|sc| sc.get("error"))
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
        .or_else(|| {
            response
                .content
                .iter()
                .find_map(Content::as_text)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "Tool execution failed".to_owned());
    if error_code == "PermissionDenied" {
        return ProtocolError::PermissionDenied {
            tool_name: tool_name.to_owned(),
            reason: message,
        };
    }
    let rendered = format!("{tool_name}: {message}");
    // A caller's mistake is a parameter error and a request the caller can
    // change is a request error; only what the caller cannot fix is internal.
    match error_code {
        "InvalidInput" | "MissingRequiredField" | "InvalidFormat" | "ValueOutOfRange" => {
            ProtocolError::InvalidParameters(rendered)
        }
        "AuthRequired"
        | "AuthInvalid"
        | "AuthExpired"
        | "NoProviderConnected"
        | "ResourceNotFound"
        | "RateLimitExceeded"
        | "QuotaExceeded"
        | "ExternalRateLimited" => ProtocolError::InvalidRequest(rendered),
        // An upstream service's refusal ("route search is unavailable, retry
        // in a few minutes", "intervals.icu answered 503") is written for the
        // caller — `AppError::tool_message` passes it — so it stays a failure
        // whose text the transport shows, never an internal error whose text
        // only the log keeps.
        "ExternalServiceError" | "ExternalServiceUnavailable" | "ExternalAuthFailed" => {
            ProtocolError::ExecutionFailed(rendered)
        }
        _ => ProtocolError::InternalError(rendered),
    }
}

/// Convert a tool's [`ToolResponse`] into a [`UniversalResponse`].
///
/// Preserves the success bit, the structured JSON payload (`structuredContent`),
/// and the error text so protocol clients see the same shape regardless of which
/// direction the dispatch came from.
pub(super) fn tool_response_to_universal_response(
    tool_name: &str,
    response: &ToolResponse,
) -> UniversalResponse {
    if response.is_error {
        let message = response
            .structured_content
            .as_ref()
            .and_then(|sc| sc.get("error"))
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned)
            .or_else(|| {
                response
                    .content
                    .iter()
                    .find_map(Content::as_text)
                    .map(ToOwned::to_owned)
            })
            .unwrap_or_else(|| "Tool execution failed".to_owned());
        UniversalResponse {
            success: false,
            result: response.structured_content.clone(),
            error: Some(message),
            metadata: None,
        }
    } else {
        tracing::debug!(tool_name, "universal executor: tool executed successfully");
        UniversalResponse {
            success: true,
            result: response.structured_content.clone(),
            error: None,
            metadata: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversions::tool_result_to_response;
    use pierre_core::errors::AppError;

    /// A provider or upstream service's refusal is written for the caller
    /// (`AppError::tool_message` passes it), so it must come back as a
    /// failure whose text the transport shows, never as an internal error
    /// whose text is kept in the log.
    #[test]
    fn an_upstream_refusal_keeps_its_message_for_the_caller() {
        for error in [
            AppError::new(
                ErrorCode::ExternalServiceUnavailable,
                "Route search is temporarily unavailable: retry in a few minutes.",
            ),
            AppError::external_service("intervals_icu", "answered HTTP 503"),
            AppError::new(ErrorCode::ExternalAuthFailed, "Strava refused the token"),
        ] {
            let code = format!("{:?}", error.code);
            let expected = error.tool_message();
            let response = tool_result_to_response(Err(error));
            let raised = raised_error_code(&response).expect("a raised error is tagged");
            assert_eq!(raised, code, "bug: the raised code is the error's own");
            match protocol_error_from_raised("discover_routes", &raised, &response) {
                ProtocolError::ExecutionFailed(message) => {
                    assert_eq!(message, format!("discover_routes: {expected}"));
                }
                other => panic!("bug: {code} became {other:?}"),
            }
        }
    }

    #[test]
    fn a_server_fault_stays_internal() {
        let response = tool_result_to_response(Err(AppError::database("relation goals")));
        let raised = raised_error_code(&response).expect("a raised error is tagged");
        let mapped = protocol_error_from_raised("set_goal", &raised, &response);
        assert!(
            matches!(
                &mapped,
                ProtocolError::InternalError(message) if !message.contains("relation goals")
            ),
            "bug: a database fault must stay internal, its detail kept from the caller: \
             {mapped:?}"
        );
    }
}
