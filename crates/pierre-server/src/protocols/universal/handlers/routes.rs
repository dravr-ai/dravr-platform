// ABOUTME: Universal-executor handler for the discover_routes MCP tool
// ABOUTME: Bridges DiscoverRoutesTool (McpTool trait) into UniversalExecutor's registry

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::future::Future;
use std::pin::Pin;

use serde_json::json;
use tracing::warn;

use pierre_core::models::TenantId;
use uuid::Uuid;

use crate::errors::AppResult;
use crate::protocols::universal::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::protocols::ProtocolError;
use crate::tools::context::{AuthMethod, ToolExecutionContext};
use crate::tools::implementations::routes::DiscoverRoutesTool;
use crate::tools::result::ToolResult;
use crate::tools::traits::McpTool;
use crate::utils::uuid::parse_user_id_for_protocol;

/// Handle the `discover_routes` tool — resolve a place or coordinates via
/// Nominatim and return real named OSM routes within the requested radius.
///
/// Delegates to [`DiscoverRoutesTool::execute`] so the Overpass-backed
/// route discovery logic lives in a single place and the universal path
/// stays in lockstep with the MCP path.
#[must_use]
pub fn handle_discover_routes(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;
        let tenant_id = if let Some(raw) = request.tenant_id.as_deref() {
            let uuid = Uuid::parse_str(raw).map_err(|e| {
                ProtocolError::InvalidRequest(format!("Invalid tenant_id format: {e}"))
            })?;
            TenantId::from_uuid(uuid)
        } else {
            warn!(
                user_id = %user_uuid,
                "discover_routes: request missing tenant_id — rejecting"
            );
            return Err(ProtocolError::InvalidRequest(
                "tenant_id is required for discover_routes".to_owned(),
            ));
        };

        let context = ToolExecutionContext::new(
            user_uuid,
            Some(tenant_id),
            executor.resources.clone(),
            AuthMethod::JwtBearer,
        );

        let tool = DiscoverRoutesTool;
        let result = tool.execute(request.parameters.clone(), &context).await;
        Ok(tool_result_to_universal_response(result))
    })
}

/// Convert a [`ToolResult`] returned by an [`McpTool::execute`] call into
/// a [`UniversalResponse`] the executor expects.
///
/// Preserves the success bit, the structured JSON payload for success
/// (so the LLM sees the route list verbatim), and the error text plus
/// payload for failures (so the LLM sees `{"error": "..."}`).
fn tool_result_to_universal_response(result: AppResult<ToolResult>) -> UniversalResponse {
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
            warn!(error = %e, "discover_routes: tool execution failed with AppError");
            UniversalResponse {
                success: false,
                result: Some(json!({ "error": e.to_string() })),
                error: Some(format!("discover_routes failed: {e}")),
                metadata: None,
            }
        }
    }
}
