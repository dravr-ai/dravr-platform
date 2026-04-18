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
    let request = build_universal_request(ctx, args, tool_name)?;
    let executor = UniversalExecutor::new(ctx.resources.clone());
    let outcome = handler(&executor, request).await;
    Ok(universal_response_to_tool_result(tool_name, outcome))
}

/// Build a [`UniversalRequest`] from the tool-execution context + tool args.
///
/// Fails fast when the context has no tenant — handlers rely on tenant
/// isolation and silently defaulting would be a multi-tenancy bug.
fn build_universal_request(
    ctx: &ToolExecutionContext,
    args: Value,
    tool_name: &'static str,
) -> AppResult<UniversalRequest> {
    let tenant_id = ctx.tenant_id.ok_or_else(|| {
        AppError::auth_invalid(format!(
            "{tool_name}: tool execution context missing tenant_id"
        ))
    })?;

    Ok(UniversalRequest {
        tool_name: tool_name.to_owned(),
        parameters: args,
        user_id: ctx.user_id.to_string(),
        protocol: "chat".to_owned(),
        tenant_id: Some(tenant_id.to_string()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    })
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
            let content = response.result.unwrap_or_else(|| {
                json!({
                    "error": response
                        .error
                        .unwrap_or_else(|| format!("{tool_name} failed"))
                })
            });
            ToolResult::error(content)
        }
        Err(e) => ToolResult::error(json!({
            "error": format!("{tool_name} failed: {e}"),
        })),
    }
}
