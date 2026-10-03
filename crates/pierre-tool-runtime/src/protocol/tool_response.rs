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
/// to `InvalidParameters`; auth / provider gating maps to `InvalidRequest`; an
/// authorization refusal maps to `PermissionDenied`, which carries the tool
/// name itself; every other code (not-found, internal, …) maps to
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
    match error_code {
        "InvalidInput" => ProtocolError::InvalidParameters(rendered),
        "AuthRequired" | "AuthInvalid" | "AuthExpired" | "NoProviderConnected" => {
            ProtocolError::InvalidRequest(rendered)
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
