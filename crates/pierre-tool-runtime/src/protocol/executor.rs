// ABOUTME: Clean universal executor that coordinates authentication, routing, and execution
// ABOUTME: Replaces monolithic universal.rs with composable services and type-safe routing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::auth::AuthService;
use crate::context::AuthMethod;
use crate::conversions::RAISED_ERROR_CODE_KEY;
use crate::protocol::types::{UniversalRequest, UniversalResponse};
use crate::protocols::ProtocolError;
use crate::runtime::ToolRuntime;
use dravr_tronc::mcp::schema::{Content, ToolResponse};
use dravr_tronc::mcp::tool::ToolContext;
use pierre_config::constants::time_constants::SECONDS_PER_HOUR_F64;
use pierre_core::models::{Activity, TenantId};
use pierre_core::uuid_utils::parse_user_id_for_protocol;
use pierre_intelligence::physiological_constants::business_thresholds::{
    DEFAULT_HR_EFFORT_SCORE, DISTANCE_SCORE_DIVISOR, DURATION_SCORE_FACTOR, MAX_SCORE,
    MIN_VALID_DISTANCE,
};
use pierre_intelligence::physiological_constants::efficiency_defaults::{
    DEFAULT_EFFICIENCY_SCORE, DEFAULT_EFFICIENCY_WITH_DISTANCE,
};
use pierre_intelligence::IntelligenceConfig;
use std::sync::Arc;
use uuid::Uuid;

/// Intelligence service interface for analysis operations
/// Provides abstraction layer for future intelligence module integration
pub struct IntelligenceService;

impl IntelligenceService {
    /// Creates a new intelligence service instance
    #[must_use]
    pub fn new(_resources: Arc<dyn ToolRuntime>) -> Self {
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
/// `resources.tool_registry()`, and execution goes straight through
/// `McpTool::execute`. `UniversalExecutor` exists now only to host the
/// `auth_service` / `intelligence_service` lifecycles that the chat /
/// A2A / SSE callers share.
pub struct UniversalExecutor {
    /// Authentication service for handling OAuth and token validation
    pub auth_service: AuthService,
    /// Intelligence service for activity analysis and insights
    pub intelligence_service: IntelligenceService,
    /// Shared server resources (database, weather service, etc.)
    pub resources: Arc<dyn ToolRuntime>,
}

impl UniversalExecutor {
    /// Create new executor with all services
    #[must_use]
    pub fn new(resources: Arc<dyn ToolRuntime>) -> Self {
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
        self.resources.cageux_config_registry().current()
    }

    /// Dispatch a tool call to the unified `McpTool` registry.
    ///
    /// Single dispatch path: every tool — regardless of which protocol
    /// surfaced the request (MCP over HTTP/SSE/stdio, chat tool loop, A2A,
    /// SSE subscription) — resolves to exactly one `McpTool::execute` body.
    /// There is no parallel fn-pointer registry anymore; tools that used
    /// to exist only as `handle_*` functions now have `McpTool` impls that
    /// delegate to those handlers via `tools::dispatch`.
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
            .tool_registry()
            .get(&request.tool_name)
            .cloned()
            .ok_or_else(|| ProtocolError::ToolNotFound {
                tool_id: request.tool_name.clone(),
                available_count: self.resources.tool_registry().tool_names().len(),
            })?;

        let ctx = build_tool_context(&self.resources, &request).await?;
        let args = request.parameters;
        let tool_name = request.tool_name;

        let response = tool.execute(&self.resources, &ctx, args).await;

        if response.is_error {
            // Preserve the provider-auth short-circuit across the tronc
            // `ToolResponse` boundary: a tool body that hit `ProviderAuthRequired`
            // encodes the provider slug in `structured_content` so the chat tool
            // loop can mint a hosted-login URL instead of surfacing a generic
            // failure to the LLM.
            if let Some(provider) = provider_auth_required_slug(&response) {
                return Err(ProtocolError::ProviderAuthRequired { provider });
            }

            // A body that returned `Err(AppError)` ("the tool refused to run")
            // tagged the response with its originating `ErrorCode`. Re-raise it
            // as the matching `ProtocolError` so `.is_err()` callers see input /
            // auth / tenant failures as protocol errors — the pre-E3 contract.
            // A body that returned `Ok(ToolResult::error(..))` carries no tag and
            // falls through to the in-band `success: false` response below.
            if let Some(code) = raised_error_code(&response) {
                return Err(protocol_error_from_raised(&tool_name, &code, &response));
            }
        }

        Ok(tool_response_to_universal_response(&tool_name, &response))
    }

    /// Check if executor has a specific tool
    #[must_use]
    pub fn has_tool(&self, tool_name: &str) -> bool {
        self.resources.tool_registry().get(tool_name).is_some()
    }
}

/// Build the per-call [`ToolContext`] handed to `McpTool::execute` from a
/// [`UniversalRequest`].
///
/// Enforces that `user_id` parses as a UUID (fail fast), and that a present
/// `tenant_id` parses as a UUID. A missing `tenant_id` passes through as
/// absent — tenant presence is enforced per-tool, not at this boundary, because
/// some tools legitimately run pre-onboarding (e.g. provider listing). Identity
/// fields are carried as strings on the host-agnostic context and rebuilt into
/// the typed [`crate::context::ToolExecutionContext`] inside each tool via
/// `ToolExecutionContext::from_tronc`.
///
/// Resolves the caller's admin flag from the global `User.is_admin` flag and
/// records it via [`ToolContext::as_admin`]. The flag is then read back as the
/// cached admin status inside each tool, so admin-gated tools see the same
/// decision regardless of which dispatch path surfaced the request.
///
/// System-admin tools (e.g. system-coach management) gate on the global
/// `is_admin` flag, not the per-tenant role: being a tenant owner makes a user
/// admin *of their tenant*, which must not grant system-wide admin powers. A
/// global-lookup failure defaults to non-admin (deny-by-default).
async fn build_tool_context(
    resources: &Arc<dyn ToolRuntime>,
    request: &UniversalRequest,
) -> Result<ToolContext, ProtocolError> {
    let user_uuid = parse_user_id_for_protocol(&request.user_id)?;

    let is_admin = resources
        .repos()
        .users
        .get_global(user_uuid)
        .await
        .ok()
        .flatten()
        .is_some_and(|user| user.is_admin);

    let mut ctx = ToolContext::new()
        .with_user(user_uuid.to_string())
        .with_auth_method(AuthMethod::JwtBearer.as_str())
        .as_admin(is_admin);

    if let Some(raw) = request.tenant_id.as_deref() {
        let uuid = Uuid::parse_str(raw)
            .map_err(|e| ProtocolError::InvalidRequest(format!("Invalid tenant_id format: {e}")))?;
        ctx = ctx.with_tenant(TenantId::from_uuid(uuid).to_string());
    }

    Ok(ctx)
}

/// Detect the provider-auth sentinel a tool body encodes in `structured_content`
/// when it hit `ProviderAuthRequired`, returning the provider slug to re-raise.
fn provider_auth_required_slug(response: &ToolResponse) -> Option<String> {
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

/// Read the `ErrorCode` tag a raised [`pierre_core::errors::AppError`] recorded
/// in `structured_content` (see [`RAISED_ERROR_CODE_KEY`]). Returns `None` for an
/// in-band `Ok(ToolResult::error(..))` failure, which carries no tag.
fn raised_error_code(response: &ToolResponse) -> Option<String> {
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
/// to `InvalidParameters`; auth / provider gating maps to `InvalidRequest`; every
/// other code (permission, not-found, internal, …) maps to `InternalError`. The
/// `tool_name` prefix and original message are preserved so message-asserting
/// callers keep matching.
fn protocol_error_from_raised(
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
fn tool_response_to_universal_response(
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
