// ABOUTME: Generic bridge from the McpTool trait into UniversalExecutor handlers
// ABOUTME: Lets every register_*_tools() site reuse one pattern instead of hand-rolled wrappers

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::future::Future;
use std::pin::Pin;

use pierre_core::models::TenantId;
use serde_json::json;
use tracing::warn;
use uuid::Uuid;

use crate::errors::AppResult;
use crate::protocols::universal::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::protocols::ProtocolError;
use crate::tools::context::{AuthMethod, ToolExecutionContext};
use crate::tools::result::ToolResult;
use crate::tools::traits::McpTool;
use crate::utils::uuid::parse_user_id_for_protocol;

/// Parse + validate request identity, build a [`ToolExecutionContext`].
///
/// Common preamble shared by every bridged MCP tool handler.
fn build_context(
    executor: &UniversalToolExecutor,
    request: &UniversalRequest,
    tool_name: &'static str,
) -> Result<ToolExecutionContext, ProtocolError> {
    let user_uuid = parse_user_id_for_protocol(&request.user_id)?;
    let tenant_id = if let Some(raw) = request.tenant_id.as_deref() {
        let uuid = Uuid::parse_str(raw).map_err(|e| {
            ProtocolError::InvalidRequest(format!("{tool_name}: invalid tenant_id format: {e}"))
        })?;
        TenantId::from_uuid(uuid)
    } else {
        warn!(
            tool_name,
            user_id = %user_uuid,
            "request missing tenant_id — rejecting"
        );
        return Err(ProtocolError::InvalidRequest(format!(
            "tenant_id is required for {tool_name}"
        )));
    };

    Ok(ToolExecutionContext::new(
        user_uuid,
        Some(tenant_id),
        executor.resources.clone(),
        AuthMethod::JwtBearer,
    ))
}

/// Convert a [`ToolResult`] into a [`UniversalResponse`].
///
/// Preserves the success bit, the structured JSON payload for success
/// (so the LLM sees real tool output verbatim), and the error text plus
/// payload for failures (so the LLM sees `{"error": "..."}`).
fn tool_result_to_universal_response(
    tool_name: &'static str,
    result: AppResult<ToolResult>,
) -> UniversalResponse {
    match result {
        Ok(tool_result) => {
            if tool_result.is_error {
                let message = tool_result
                    .content
                    .get("error")
                    .and_then(|v| v.as_str())
                    .map_or_else(|| tool_result.content.to_string(), ToOwned::to_owned);
                UniversalResponse {
                    success: false,
                    result: Some(tool_result.content),
                    error: Some(message),
                    metadata: None,
                }
            } else {
                UniversalResponse {
                    success: true,
                    result: Some(tool_result.content),
                    error: None,
                    metadata: None,
                }
            }
        }
        Err(e) => {
            warn!(tool_name, error = %e, "tool execution failed with AppError");
            UniversalResponse {
                success: false,
                result: Some(json!({ "error": e.to_string() })),
                error: Some(format!("{tool_name} failed: {e}")),
                metadata: None,
            }
        }
    }
}

/// Bridge an [`McpTool`] implementation into a `UniversalExecutor` handler
/// future.
///
/// Usage — wire a new tool into `executor.rs::register_*_tools` with:
///
/// ```text
/// registry.register(ToolInfo::async_tool(ToolId::VerifyClaim, |exec, req| {
///     Box::pin(bridge_mcp_tool(exec, req, VerifyClaimTool, "verify_claim"))
/// }));
/// ```
///
/// The tool instance is constructed at call time rather than kept in a
/// registry so each invocation starts with a clean, un-shared state —
/// mirroring how the MCP dispatch path already instantiates tools per-call.
pub fn bridge_mcp_tool<'a, T>(
    executor: &'a UniversalToolExecutor,
    request: UniversalRequest,
    tool: T,
    tool_name: &'static str,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + 'a>>
where
    T: McpTool + Send + Sync + 'static,
{
    Box::pin(async move {
        let context = build_context(executor, &request, tool_name)?;
        let result = tool.execute(request.parameters.clone(), &context).await;
        Ok(tool_result_to_universal_response(tool_name, result))
    })
}
