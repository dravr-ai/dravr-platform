// ABOUTME: Inverse of protocols::universal::handlers::mcp_bridge — lets McpTool::execute delegate to a UniversalExecutor handler fn
// ABOUTME: Collapses dispatch drift between MCP protocol and chat-pipeline tool loops to a single execution path

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Delegate an [`McpTool::execute`] body to a `UniversalExecutor` handler.
//!
//! Used during the tool-registry unification: one `McpTool` impl per tool,
//! dispatched through the same registry from both MCP protocol and chat
//! pipeline paths. Handler bodies still live under
//! `protocols::universal::handlers::`, and this helper bridges the
//! canonical `McpTool` impl onto them so exactly one body executes per tool
//! regardless of which protocol dispatched it.
//!
//! Retired once handler inlining completes (Stage 5 of the unification) —
//! the helper and every call site can then be removed.

use std::future::Future;
use std::pin::Pin;

use serde_json::{json, Value};

use crate::errors::{AppError, AppResult};
use crate::protocols::universal::{UniversalExecutor, UniversalRequest, UniversalResponse};
use crate::protocols::ProtocolError;
use crate::tools::context::ToolExecutionContext;
use crate::tools::result::ToolResult;

/// Handler signature every `protocols::universal::handlers::handle_*` fn conforms to.
///
/// Each handler takes `&UniversalExecutor` + `UniversalRequest` and returns a
/// pinned future resolving to a protocol response.
pub type HandlerFn = for<'a> fn(
    &'a UniversalExecutor,
    UniversalRequest,
) -> Pin<
    Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + 'a>,
>;

/// Delegate the body of an `McpTool::execute(args, ctx)` call to a
/// `UniversalExecutor` handler.
///
/// Builds a [`UniversalRequest`] from the tool-execution context, spins up a
/// lightweight `UniversalExecutor` from the shared [`ServerResources`],
/// invokes the handler, and converts the [`UniversalResponse`] back into a
/// [`ToolResult`]. The tool name is threaded through solely for logging and
/// error-message attribution.
///
/// # Errors
///
/// Returns `AppError::auth_invalid` when the context is missing a tenant
/// (handlers require it) and `AppError::external_service` when the handler
/// returns a `ProtocolError`.
pub async fn delegate_to_handler(
    ctx: &ToolExecutionContext,
    args: Value,
    tool_name: &'static str,
    handler: HandlerFn,
) -> AppResult<ToolResult> {
    let request = build_universal_request(ctx, args, tool_name);
    let executor = UniversalExecutor::new(ctx.resources.clone());
    match handler(&executor, request).await {
        // Success path: map UniversalResponse → ToolResult (success OR business
        // error like "no activities found" that the handler chose to surface as
        // `success: false`).
        Ok(response) => Ok(universal_response_to_tool_result(tool_name, Ok(response))),
        // Protocol-level failure (missing required arg, invalid tenant, auth
        // denied, …) bubbles back as AppError so the caller's Result semantics
        // match what the UniversalExecutor dispatch returns directly from the
        // handler. Without this, a `.is_err()` check at the tool call site
        // becomes false positives for every validation failure.
        Err(e) => Err(protocol_error_to_app_error(tool_name, &e)),
    }
}

/// Map a [`ProtocolError`] surfaced by a handler into the [`AppError`] shape
/// `McpTool::execute` callers expect — preserving the distinction between
/// "invalid input" (maps to `AppError::invalid_input`) and "something broke
/// internally" (maps to `AppError::internal`).
fn protocol_error_to_app_error(tool_name: &'static str, e: &ProtocolError) -> AppError {
    // Preserve the original variant's Display (e.g. "Invalid parameters: ...",
    // "Missing parameter: ...") so test assertions and user-facing error text
    // stay meaningful through the mapping.
    let rendered = format!("{tool_name}: {e}");
    match e {
        ProtocolError::InvalidRequest(_)
        | ProtocolError::InvalidParameters(_)
        | ProtocolError::InvalidParameter { .. }
        | ProtocolError::MissingParameter { .. }
        | ProtocolError::ToolNotFound { .. } => AppError::invalid_input(rendered),
        _ => AppError::internal(rendered),
    }
}

/// Build a [`UniversalRequest`] from the tool-execution context + tool args.
///
/// `tenant_id` is threaded through when present — not required at this layer
/// because some read-only catalog tools (yoga pose lookups, stretching
/// exercise catalogs, etc.) are tenant-agnostic. Per-tool tenant enforcement
/// is the responsibility of the handler, which has the schema + semantics
/// to decide whether tenant scoping applies.
fn build_universal_request(
    ctx: &ToolExecutionContext,
    args: Value,
    tool_name: &'static str,
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

/// Convert a handler's [`Result<UniversalResponse, ProtocolError>`] into a
/// [`ToolResult`] that an `McpTool::execute` body can return.
///
/// Mirrors the conversion `mcp_bridge::tool_result_to_universal_response`
/// does in the opposite direction so that MCP clients and chat clients see
/// identical outputs regardless of which side originated the dispatch.
fn universal_response_to_tool_result(
    tool_name: &'static str,
    outcome: Result<UniversalResponse, ProtocolError>,
) -> ToolResult {
    match outcome {
        Ok(response) if response.success => response.result.map_or_else(
            || ToolResult::ok(json!({"status": "success"})),
            ToolResult::ok,
        ),
        Ok(response) => {
            // Handlers can populate both `result` (structured payload) and
            // `error` (human message). Callers that inspect `content["error"]`
            // would otherwise miss the message when the handler also emitted a
            // structured payload — so surface `error` as a top-level field
            // when it isn't already present in the payload.
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
            ToolResult::error(content)
        }
        Err(e) => ToolResult::error(json!({
            "error": format!("{tool_name} failed: {e}"),
        })),
    }
}
