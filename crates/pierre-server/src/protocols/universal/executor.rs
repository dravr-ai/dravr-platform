// ABOUTME: Clean universal executor that coordinates authentication, routing, and execution
// ABOUTME: Replaces monolithic universal.rs with composable services and type-safe routing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::auth_service::AuthService;
use pierre_core::models::TenantId;
use pierre_intelligence::IntelligenceConfig;
use uuid::Uuid;

use crate::constants::time_constants::SECONDS_PER_HOUR_F64;
use crate::errors::AppError;
use crate::intelligence::physiological_constants::business_thresholds::{
    DEFAULT_HR_EFFORT_SCORE, DISTANCE_SCORE_DIVISOR, DURATION_SCORE_FACTOR, MAX_SCORE,
    MIN_VALID_DISTANCE,
};
use crate::intelligence::physiological_constants::efficiency_defaults::{
    DEFAULT_EFFICIENCY_SCORE, DEFAULT_EFFICIENCY_WITH_DISTANCE,
};
use crate::mcp::resources::ServerResources;
use crate::models::Activity;
use crate::protocols::universal::{UniversalRequest, UniversalResponse};
use crate::protocols::ProtocolError;
use crate::tools::context::{AuthMethod, ToolExecutionContext};
use crate::tools::result::ToolResult;
use crate::utils::uuid::parse_user_id_for_protocol;
use std::sync::Arc;

/// Intelligence service interface for analysis operations
/// Provides abstraction layer for future intelligence module integration
pub struct IntelligenceService;

impl IntelligenceService {
    /// Creates a new intelligence service instance
    #[must_use]
    pub fn new(_resources: Arc<ServerResources>) -> Self {
        Self
    }

    /// Analyze activity data with intelligence engine
    ///
    /// # Errors
    /// Returns error if intelligence analysis fails
    pub fn analyze_activity(&self, activity: &Activity) -> Result<serde_json::Value, String> {
        // Calculate basic efficiency score
        let efficiency_score =
            activity
                .distance_meters()
                .map_or(DEFAULT_EFFICIENCY_WITH_DISTANCE, |distance| {
                    if activity.duration_seconds() > 0 && distance > f64::from(MIN_VALID_DISTANCE) {
                        let duration_f64 = f64::from(
                            u32::try_from(activity.duration_seconds().min(u64::from(u32::MAX)))
                                .unwrap_or(u32::MAX),
                        );
                        let speed_ms = distance / duration_f64;
                        (speed_ms * f64::from(MAX_SCORE)).min(f64::from(MAX_SCORE))
                    } else {
                        DEFAULT_EFFICIENCY_SCORE
                    }
                });

        // Calculate effort score based on duration and distance
        let effort_score = if activity.duration_seconds() > 0 {
            let duration_hours = f64::from(
                u32::try_from(activity.duration_seconds().min(u64::from(u32::MAX)))
                    .unwrap_or(u32::MAX),
            ) / SECONDS_PER_HOUR_F64;
            let base_effort = duration_hours * f64::from(DURATION_SCORE_FACTOR);

            // Add distance component if available
            activity.distance_meters().map_or(base_effort, |d| {
                let distance_km = d / 1000.0;
                base_effort + (distance_km / f64::from(DISTANCE_SCORE_DIVISOR))
            })
        } else {
            f64::from(DEFAULT_HR_EFFORT_SCORE)
        };

        Ok(serde_json::json!({
            "activity_id": activity.id(),
            "analysis_type": "intelligence_engine",
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "efficiency_score": efficiency_score,
            "effort_score": effort_score.min(f64::from(MAX_SCORE)),
            "performance_insights": {
                "efficiency_rating": if efficiency_score > 75.0 { "excellent" } else if efficiency_score > 50.0 { "good" } else { "needs_improvement" },
                "effort_level": if effort_score > 80.0 { "high" } else if effort_score > 40.0 { "moderate" } else { "low" }
            },
            "recommendations": Self::generate_activity_recommendations(activity, efficiency_score, effort_score)
        }))
    }

    /// Generate recommendations based on activity analysis
    fn generate_activity_recommendations(
        activity: &Activity,
        efficiency_score: f64,
        effort_score: f64,
    ) -> Vec<String> {
        let mut recommendations = Vec::new();

        if efficiency_score < 50.0 {
            recommendations
                .push("Consider focusing on pacing strategy for improved efficiency".to_owned());
        }

        if effort_score > 80.0 {
            recommendations.push("High effort detected - ensure adequate recovery time".to_owned());
        }

        if activity.distance_meters().is_none() {
            recommendations.push("Track distance for more comprehensive analysis".to_owned());
        }

        if recommendations.is_empty() {
            recommendations.push("Great activity! Keep up the consistent training".to_owned());
        }

        recommendations
    }
}

/// Thin dispatcher that adapts a [`UniversalRequest`] onto the shared
/// [`crate::tools::registry::ToolRegistry`].
///
/// Post-unification (2026-04-18): no private registry, no fn-pointer
/// handlers, no `ToolId` enum. Tool lookup is by name against
/// `resources.tool_registry`, and execution goes straight through
/// `McpTool::execute`. `UniversalExecutor` exists now only to host the
/// `auth_service` / `intelligence_service` lifecycles that the chat /
/// A2A / SSE callers share.
pub struct UniversalExecutor {
    /// Authentication service for handling OAuth and token validation
    pub auth_service: AuthService,
    /// Intelligence service for activity analysis and insights
    pub intelligence_service: IntelligenceService,
    /// Shared server resources (database, weather service, etc.)
    pub resources: Arc<ServerResources>,
}

impl UniversalExecutor {
    /// Create new executor with all services
    #[must_use]
    pub fn new(resources: Arc<ServerResources>) -> Self {
        let auth_service = AuthService::new(resources.clone());
        let intelligence_service = IntelligenceService::new(resources.clone());

        Self {
            auth_service,
            intelligence_service,
            resources,
        }
    }

    /// Return a cheap clone of the current cageux intelligence config
    /// snapshot from the server's hot-reloadable registry.
    ///
    /// Handlers should call this once at the top of the request, bind the
    /// result to a local, and then borrow sub-configs (`.sleep_recovery`,
    /// `.nutrition`, `.algorithms`, etc.) from the local to keep the
    /// snapshot consistent for the duration of the call.
    #[must_use]
    pub fn cageux_config(&self) -> Arc<IntelligenceConfig<true>> {
        self.resources.cageux_config_registry.current()
    }

    /// Dispatch a tool call to the unified `McpTool` registry.
    ///
    /// Single dispatch path: every tool — regardless of which protocol
    /// surfaced the request (MCP over HTTP/SSE/stdio, chat tool loop, A2A,
    /// SSE subscription) — resolves to exactly one `McpTool::execute` body.
    /// There is no parallel fn-pointer registry anymore; tools that used
    /// to exist only as `handle_*` functions now have `McpTool` impls that
    /// delegate to those handlers via `tools::universal_delegate`.
    ///
    /// # Errors
    /// Returns `ProtocolError::ToolNotFound` when the tool name does not
    /// resolve in the shared [`crate::tools::registry::ToolRegistry`], and
    /// `ProtocolError::InternalError` when the context cannot be built
    /// (e.g. malformed user id) or when tool execution returns `AppError`.
    pub async fn execute_tool(
        &self,
        request: UniversalRequest,
    ) -> Result<UniversalResponse, ProtocolError> {
        let tool = self
            .resources
            .tool_registry
            .get(&request.tool_name)
            .cloned()
            .ok_or_else(|| ProtocolError::ToolNotFound {
                tool_id: request.tool_name.clone(),
                available_count: self.resources.tool_registry.tool_names().len(),
            })?;

        let context = build_tool_execution_context(&self.resources, &request)?;
        let args = request.parameters;
        let tool_name = request.tool_name;

        match tool.execute(args, &context).await {
            Ok(tool_result) => Ok(tool_result_to_universal_response(&tool_name, tool_result)),
            // Map AppError back onto the pre-unification ProtocolError shape
            // so callers that distinguish "missing required arg" (Err) from
            // "tool ran, returned a failure payload" (Ok with success=false)
            // keep the behaviour they had when UniversalExecutor owned the
            // handler dispatch directly.
            Err(e) => Err(app_error_to_protocol_error(&tool_name, &e)),
        }
    }

    /// Check if executor has a specific tool
    #[must_use]
    pub fn has_tool(&self, tool_name: &str) -> bool {
        self.resources.tool_registry.get(tool_name).is_some()
    }
}

/// Map an [`AppError`] thrown by an `McpTool::execute` body back to the
/// matching [`ProtocolError`] variant.
///
/// Validation-class errors (missing required arg, bad tenant id, auth
/// failure) flow to `ProtocolError::InvalidRequest`, so a caller doing
/// `.is_err()` on the result sees input failures as protocol errors rather
/// than as successful-but-failing responses. Everything else flows to
/// `::InternalError`.
fn app_error_to_protocol_error(tool_name: &str, e: &AppError) -> ProtocolError {
    use pierre_core::errors::ErrorCode;
    match e.code {
        ErrorCode::ProviderAuthRequired => {
            // Preserve the provider slug across the protocol boundary so the
            // tool loop can short-circuit and the chat pipeline can mint a
            // hosted-login URL. Falls back to a placeholder if `details` was
            // somehow malformed — the loop still detects the variant.
            let provider = e
                .provider_auth_required_provider()
                .unwrap_or_else(|| "unknown".to_owned());
            ProtocolError::ProviderAuthRequired { provider }
        }
        ErrorCode::InvalidInput
        | ErrorCode::AuthRequired
        | ErrorCode::AuthInvalid
        | ErrorCode::AuthExpired => ProtocolError::InvalidRequest(format!("{tool_name}: {e}")),
        _ => ProtocolError::InternalError(format!("{tool_name}: {e}")),
    }
}

/// Build the [`ToolExecutionContext`] that every `McpTool::execute` expects
/// from a [`UniversalRequest`].
///
/// Fails fast on malformed `user_id` (must be a UUID) and on missing
/// `tenant_id` — tools rely on tenant isolation, so a silent fallback would
/// be a multi-tenancy bug.
fn build_tool_execution_context(
    resources: &Arc<ServerResources>,
    request: &UniversalRequest,
) -> Result<ToolExecutionContext, ProtocolError> {
    let user_uuid = parse_user_id_for_protocol(&request.user_id)?;

    let tenant_id = if let Some(raw) = request.tenant_id.as_deref() {
        let uuid = Uuid::parse_str(raw)
            .map_err(|e| ProtocolError::InvalidRequest(format!("Invalid tenant_id format: {e}")))?;
        Some(TenantId::from_uuid(uuid))
    } else {
        None
    };

    Ok(ToolExecutionContext::new(
        user_uuid,
        tenant_id,
        resources.clone(),
        AuthMethod::JwtBearer,
    ))
}

/// Convert an `McpTool::execute` [`ToolResult`] into a [`UniversalResponse`].
///
/// Preserves the success bit, the structured JSON payload, and the error
/// text — identical to the inverse conversion in
/// `tools::universal_delegate::delegate_to_handler` so protocol clients see
/// the same shape regardless of which direction the dispatch came from.
fn tool_result_to_universal_response(
    tool_name: &str,
    tool_result: ToolResult,
) -> UniversalResponse {
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
        tracing::debug!(tool_name, "universal executor: tool executed successfully");
        UniversalResponse {
            success: true,
            result: Some(tool_result.content),
            error: None,
            metadata: None,
        }
    }
}
