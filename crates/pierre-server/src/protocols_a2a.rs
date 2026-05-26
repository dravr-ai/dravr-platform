// ABOUTME: A2A-coupled protocol converter functions split from the cross-crate ProtocolConverter
// ABOUTME: Lives in pierre-server because A2ARequest / A2AResponse types remain crate-local
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A2A protocol converter functions.
//!
//! The non-a2a converter surface (`mcp_to_universal`, `universal_to_mcp`,
//! `detect_protocol`, `tool_to_*_format`) moved to
//! `pierre_tool_runtime::protocols::ProtocolConverter`. The two functions that
//! touch `crate::a2a::*` (`A2ARequest`, `A2AResponse`, `A2AErrorResponse`)
//! stay here as free functions so the cross-crate converter doesn't need to
//! depend on the pierre-server-internal a2a module.
//!
//! Callers that used `ProtocolConverter::a2a_to_universal(...)` now call
//! `crate::protocols_a2a::a2a_to_universal(...)`.

use crate::a2a::{A2AErrorResponse, A2ARequest, A2AResponse};
use pierre_tool_runtime::protocols::{
    ProtocolError, ProtocolType, UniversalRequest, UniversalResponse,
};
use serde_json::Value;

/// Convert A2A request to universal format
///
/// # Errors
///
/// Returns an error if the A2A request has an unsupported method or if the tool name is not found in the parameters.
pub fn a2a_to_universal(
    request: &A2ARequest,
    user_id: &str,
    tenant_id: Option<String>,
) -> Result<UniversalRequest, ProtocolError> {
    // Extract tool name from A2A method
    let tool_name = match request.method.as_str() {
        "a2a/tools/call" => {
            // Tool name should be in parameters
            request
                .params
                .as_ref()
                .and_then(|p| p.get("tool"))
                .and_then(|t| t.as_str())
                .ok_or_else(|| {
                    ProtocolError::InvalidParameters("Tool name not found in A2A request".into())
                })?
                .to_owned()
        }
        _method => {
            return Err(ProtocolError::ConversionFailed {
                from: ProtocolType::A2A,
                to: ProtocolType::A2A,
                reason: "unsupported A2A method",
            });
        }
    };

    // Extract parameters
    let parameters = request
        .params
        .as_ref()
        .and_then(|p| p.get("arguments"))
        .cloned()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

    Ok(UniversalRequest {
        tool_name,
        parameters,
        user_id: user_id.to_owned(),
        protocol: "a2a".into(),
        tenant_id,
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    })
}

/// Convert universal response to A2A format
#[must_use]
pub fn universal_to_a2a(response: UniversalResponse, request_id: Option<Value>) -> A2AResponse {
    if response.success {
        A2AResponse {
            jsonrpc: "2.0".into(),
            result: response.result,
            error: None,
            id: request_id,
        }
    } else {
        A2AResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(A2AErrorResponse {
                code: -32603,
                message: response.error.unwrap_or_else(|| "Internal error".into()),
                data: None,
            }),
            id: request_id,
        }
    }
}
