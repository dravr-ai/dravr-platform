// ABOUTME: Shared bridge between boxed-future protocol handlers and McpTool::execute.
// ABOUTME: Builds UniversalRequest envelopes and maps UniversalResponse → ToolResult.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Bridge helpers for `McpTool::execute` impls that delegate to the
//! per-tool `inner` modules or shared fitness-support helpers that still
//! return `UniversalResponse` shapes.
//!
//! Three free functions are exposed:
//!
//! - [`build_universal_request`] constructs the `UniversalRequest` envelope
//!   from a tool-execution context and the raw `args` JSON (used by the
//!   analytics `inner::*` handlers that still take `UniversalRequest`).
//! - [`map_universal_response`] converts a handler's
//!   `Result<UniversalResponse, ProtocolError>` into the `AppResult<ToolResult>`
//!   shape expected by [`McpTool::execute`].
//! - [`map_protocol_result`] lifts a generic `Result<T, ProtocolError>`
//!   into `AppResult<T>` so `McpTool::execute` bodies can use `?` for
//!   short-circuit propagation when calling fitness-support helpers
//!   directly without going through a full `UniversalResponse`.

use serde_json::{json, Value};

use crate::context::ToolExecutionContext;
use crate::protocol::{UniversalRequest, UniversalResponse};
use crate::protocols::ProtocolError;
use pierre_core::errors::{AppError, AppResult};
use pierre_tools_core::ToolResult;

/// Construct a `UniversalRequest` from the chat-pipeline tool-execution context.
///
/// All `McpTool::execute` invocations originate from the chat protocol path,
/// so `protocol` is hard-coded to `"chat"` and the progress/cancellation
/// fields are left unset.
pub(super) fn build_universal_request(
    ctx: &ToolExecutionContext,
    args: Value,
    tool_name: &str,
) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool_name.to_owned(),
        parameters: args,
        user_id: ctx.user_id.to_string(),
        protocol: "chat".to_owned(),
        tenant_id: ctx.tenant_id.map(|t| t.to_string()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    }
}

/// Lift a `Result<T, ProtocolError>` returned by a fitness-support helper
/// into `AppResult<T>` so an `McpTool::execute` body can use `?` for
/// short-circuit propagation. Validation-class protocol errors map to
/// `AppError::invalid_input` (matching the chat-loop's "missing required
/// arg" treatment); an authorization refusal keeps its `PermissionDenied`
/// code through the canonical conversion, since a nested dispatch the
/// caller's grant does not cover is a refusal, not a server fault; everything
/// else maps to `AppError::internal`.
pub(super) fn map_protocol_result<T>(
    tool_name: &str,
    result: Result<T, ProtocolError>,
) -> AppResult<T> {
    result.map_err(|e| {
        let rendered = format!("{tool_name}: {e}");
        match e {
            ProtocolError::InvalidRequest(_)
            | ProtocolError::InvalidParameters(_)
            | ProtocolError::InvalidParameter { .. }
            | ProtocolError::MissingParameter { .. }
            | ProtocolError::ToolNotFound { .. } => AppError::invalid_input(rendered),
            ProtocolError::PermissionDenied { .. } => AppError::from(e),
            _ => AppError::internal(rendered),
        }
    })
}

/// Map a handler's `Result<UniversalResponse, ProtocolError>` into the
/// `AppResult<ToolResult>` shape expected by `McpTool::execute`.
///
/// - Successful responses with a result payload return `ToolResult::ok(payload)`.
/// - Successful responses without a payload return `ToolResult::ok({status: "success"})`.
/// - Unsuccessful responses (`success: false`) return `ToolResult::error(payload_with_error)`.
/// - `ProtocolError::Invalid*` / `MissingParameter` / `ToolNotFound` lift to
///   `AppError::invalid_input`; `PermissionDenied` keeps its code through the
///   canonical conversion; everything else lifts to `AppError::internal`.
pub(super) fn map_universal_response(
    tool_name: &str,
    result: Result<UniversalResponse, ProtocolError>,
) -> AppResult<ToolResult> {
    match result {
        Ok(response) if response.success => Ok(response.result.map_or_else(
            || ToolResult::ok(json!({ "status": "success" })),
            ToolResult::ok,
        )),
        Ok(response) => {
            let fallback_error = response
                .error
                .clone()
                .unwrap_or_else(|| format!("{tool_name} failed"));
            let content = match response.result {
                Some(mut payload) => {
                    if let (Some(obj), Some(err)) =
                        (payload.as_object_mut(), response.error.as_ref())
                    {
                        obj.entry("error".to_owned())
                            .or_insert_with(|| Value::String(err.clone()));
                    }
                    payload
                }
                None => json!({ "error": fallback_error }),
            };
            Ok(ToolResult::error(content))
        }
        Err(e) => {
            let rendered = format!("{tool_name}: {e}");
            Err(match e {
                ProtocolError::InvalidRequest(_)
                | ProtocolError::InvalidParameters(_)
                | ProtocolError::InvalidParameter { .. }
                | ProtocolError::MissingParameter { .. }
                | ProtocolError::ToolNotFound { .. } => AppError::invalid_input(rendered),
                ProtocolError::PermissionDenied { .. } => AppError::from(e),
                _ => AppError::internal(rendered),
            })
        }
    }
}
