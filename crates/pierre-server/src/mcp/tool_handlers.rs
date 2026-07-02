// ABOUTME: Tool execution handlers for MCP server tool calls and provider routing
// ABOUTME: Handles tool call routing, execution, authentication, and provider-specific operations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::multitenant::ProviderToolRouter;
use super::resources::ServerContext;
use crate::constants::{
    errors::{
        ERROR_INTERNAL_ERROR, ERROR_INVALID_PARAMS, ERROR_METHOD_NOT_FOUND,
        ERROR_RATE_LIMIT_EXCEEDED, ERROR_TOKEN_EXPIRED, ERROR_TOKEN_INVALID, ERROR_TOKEN_MALFORMED,
        ERROR_UNAUTHORIZED, MSG_TOKEN_EXPIRED, MSG_TOKEN_INVALID, MSG_TOKEN_MALFORMED,
    },
    protocol::JSONRPC_VERSION,
    tools::{CONNECT_PROVIDER, DISCONNECT_PROVIDER, GET_ACTIVITIES, GET_CONNECTION_STATUS},
};
use dravr_tronc::mcp::schema::ToolResponse;
use dravr_tronc::mcp::tool::ToolContext;
use pierre_auth::auth::AuthMethod as AuthResultMethod;
use pierre_auth::auth::AuthResult;
use pierre_auth::tenant::TenantContext;
use pierre_core::errors::AppError;
use pierre_core::models::usage::InsertLlmUsage;
use pierre_core::models::{ConversationTurnId, UserTier};
use pierre_core::models::{OAuthNotification, TenantId};
use pierre_database::backends::NotificationRepository;
use pierre_mcp_schema::json_schemas;
use pierre_mcp_schema::{McpError, McpRequest, McpResponse};
use pierre_mcp_transport::tenant_isolation::extract_tenant_context_internal;
use pierre_runtime_context::{default_admin_config, AdminConfigLookup};
use pierre_services::usage_counter::UsageCounterService;
use pierre_tool_runtime::context::AuthMethod;
use pierre_tool_runtime::protocol::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::protocols::converter::ProtocolConverter;
use pierre_tool_runtime::runtime::ToolRuntime;
// Other trait methods dispatched through repos.tenants / repos.llm_usage / repos.users
use serde_json::{json, Value};
use std::fmt::Write;
use std::sync::Arc;
use std::time::Instant;
use tracing::field::{display, Empty};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Default ID for notifications and error responses that don't have a request ID
fn default_request_id() -> Value {
    Value::Number(serde_json::Number::from(0))
}

/// OAuth credentials provided in MCP requests
pub struct McpOAuthCredentials<'a> {
    /// Strava OAuth client ID
    pub strava_client_id: Option<&'a str>,
    /// Strava OAuth client secret
    pub strava_client_secret: Option<&'a str>,
    /// Fitbit OAuth client ID
    pub fitbit_client_id: Option<&'a str>,
    /// Fitbit OAuth client secret
    pub fitbit_client_secret: Option<&'a str>,
}

/// Context for routing tool calls with necessary resources and identity
///
/// Tenant context is required for all tool executions to ensure proper
/// tenant isolation and tool enablement policy enforcement. The per-call
/// [`ToolContext`] is resolved once by the transport (the tronc auth hook for
/// the HTTP path, the SSE entry for the SSE path) and threaded unchanged into
/// the tool registry.
pub struct ToolRoutingContext<'a> {
    /// Server resources for dependency injection
    pub resources: &'a Arc<ServerContext>,
    /// Tenant context for multi-tenant isolation (required)
    pub tenant_context: &'a TenantContext,
    /// Per-call identity/tenant/admin context handed to the tool registry
    pub tool_context: &'a ToolContext,
}

/// Tool execution handlers for MCP protocol
pub struct ToolHandlers;

impl ToolHandlers {
    /// Handle tools/call request with authentication from resources
    #[tracing::instrument(
        skip(request, resources),
        fields(
            method = %request.method,
            request_id = Empty,
            tool_name = Empty,
            user_id = Empty,
            tenant_id = Empty,
            success = Empty,
            duration_ms = Empty,
        )
    )]
    pub async fn handle_tools_call_with_resources(
        request: McpRequest,
        resources: &Arc<ServerContext>,
    ) -> McpResponse {
        // Record the JSON-RPC id as its inner value's Display (e.g. `3`) rather
        // than the Debug of the Option<Value> (`Some(Number(3))`) so notify
        // events inheriting this span render a clean request_id.
        if let Some(id) = &request.id {
            tracing::Span::current().record("request_id", display(id));
        }

        // Extract auth token from either HTTP Authorization header or MCP params
        let auth_token_string = request
            .params
            .as_ref()
            .and_then(|params| params.get("token"))
            .and_then(|token| token.as_str())
            .map(|mcp_token| format!("Bearer {mcp_token}"));

        let auth_token = request
            .auth_token
            .as_deref()
            .or(auth_token_string.as_deref());

        debug!(
            "MCP tool call authentication attempt for method: {} (token source: {})",
            request.method,
            if request.auth_token.is_some() {
                "HTTP header"
            } else {
                "MCP params"
            }
        );

        match resources
            .auth
            .auth_middleware
            .authenticate_request(auth_token)
            .await
        {
            Ok(auth_result) => {
                // Record authentication success in span (tenant_id recorded after extraction)
                tracing::Span::current().record("user_id", auth_result.user_id.to_string());

                info!(
                    "MCP tool call authentication successful for user: {} (method: {})",
                    auth_result.user_id,
                    auth_result.auth_method.display_name()
                );

                // Update user's last active timestamp
                if let Err(e) = resources
                    .common
                    .repos
                    .users
                    .update_last_active(auth_result.user_id)
                    .await
                {
                    warn!(
                        user_id = %auth_result.user_id,
                        error = %e,
                        "Failed to update user last active timestamp (activity tracking impacted)"
                    );
                }

                // Extract tenant context from request and auth result
                // Tenant context is REQUIRED for tool execution to ensure tenant isolation
                // Priority: JWT active_tenant_id > user's default tenant
                let tenant_context = match extract_tenant_context_internal(
                    &resources.common.repos,
                    Some(auth_result.user_id),
                    auth_result.active_tenant_id.map(TenantId::from), // Pass active_tenant_id from JWT claims
                    None, // MCP transport headers not applicable here
                )
                .await
                {
                    // Carry the JWT `jti` as the Guardian turn token so headless
                    // `/mcp` tool calls in one chat turn share a taint bucket.
                    Ok(Some(ctx)) => ctx.with_session_id(auth_result.session_id.clone()),
                    Ok(None) => {
                        // User has no tenant membership - cannot execute tools
                        warn!(
                            user_id = %auth_result.user_id,
                            "User has no tenant membership - rejecting tool execution"
                        );
                        return McpResponse::error_with_data(
                            request.id,
                            ERROR_UNAUTHORIZED,
                            "User must be assigned to a tenant to execute tools".to_owned(),
                            serde_json::json!({
                                "error_type": "tenant_required",
                                "user_id": auth_result.user_id.to_string()
                            }),
                        );
                    }
                    Err(e) => {
                        // Tenant extraction failed - cannot proceed safely
                        error!(
                            user_id = %auth_result.user_id,
                            error = %e,
                            "Tenant context extraction failed - rejecting tool execution"
                        );
                        return McpResponse::error_with_data(
                            request.id,
                            ERROR_INTERNAL_ERROR,
                            "Failed to extract tenant context".to_owned(),
                            serde_json::json!({
                                "error_type": "tenant_extraction_failed",
                                "detailed_error": e.to_string()
                            }),
                        );
                    }
                };

                // Record tenant context in span now that we have it
                tracing::Span::current().record("tenant_id", tenant_context.tenant_id.to_string());

                // Pre-execution quota check: shared budget with chat route
                if let Some(quota_error) = Self::check_tool_quota(
                    resources,
                    &tenant_context,
                    auth_result.user_id,
                    request.id.clone(),
                )
                .await
                {
                    return quota_error;
                }

                // Use the provided ServerContext directly
                Self::handle_tool_execution_direct(request, auth_result, tenant_context, resources)
                    .await
            }
            Err(e) => {
                tracing::Span::current().record("success", false);
                Self::handle_authentication_error(request, &e)
            }
        }
    }

    /// Execute a tool call for an already-authenticated caller (tronc HTTP path).
    ///
    /// The tronc auth hook has resolved the caller's identity into `tool_context`
    /// (and `user_id`/`tenant_id`); this runs the post-auth flow without
    /// re-authenticating: tool-enablement, quota, activity quota, routing,
    /// OAuth-notification augmentation, and usage recording. Failures (quota,
    /// disabled tool, unknown tool) are reported in-band as an error
    /// [`ToolResponse`], per the MCP `tools/call` model.
    pub async fn dispatch_tool_call(
        resources: &Arc<ServerContext>,
        _state: &Arc<dyn ToolRuntime>,
        tool_context: &ToolContext,
        user_id: Uuid,
        tenant_id: TenantId,
        tool_name: &str,
        args: Value,
    ) -> ToolResponse {
        let tenant_context = match extract_tenant_context_internal(
            &resources.common.repos,
            Some(user_id),
            Some(tenant_id),
            None,
        )
        .await
        {
            Ok(Some(ctx)) => ctx,
            Ok(None) => {
                return ToolResponse::error(
                    "User must be assigned to a tenant to execute tools".to_owned(),
                );
            }
            Err(e) => {
                error!(user_id = %user_id, error = %e, "Tenant context extraction failed");
                return ToolResponse::error("Failed to extract tenant context".to_owned());
            }
        };

        let request_id = tool_context
            .request_id
            .clone()
            .unwrap_or_else(default_request_id);

        // Shared budget + activity quota (mirrors the chat route).
        if let Some(quota_error) = Self::check_tool_quota(
            resources,
            &tenant_context,
            user_id,
            Some(request_id.clone()),
        )
        .await
        {
            return Self::mcp_response_to_tool_response(quota_error);
        }
        if let Some(error_response) = Self::check_tool_enabled(
            resources,
            &tenant_context,
            tool_name,
            Some(request_id.clone()),
        )
        .await
        {
            return Self::mcp_response_to_tool_response(error_response);
        }
        if let Some(error_response) = Self::check_activity_quota(
            resources,
            &tenant_context,
            user_id,
            tool_name,
            &args,
            Some(request_id.clone()),
        )
        .await
        {
            return Self::mcp_response_to_tool_response(error_response);
        }

        let start_time = Instant::now();
        let routing_context = ToolRoutingContext {
            resources,
            tenant_context: &tenant_context,
            tool_context,
        };
        let response =
            Self::route_tool_call(tool_name, &args, request_id, user_id, &routing_context).await;

        let response = Self::append_oauth_notifications_to_response(
            response,
            user_id,
            tool_name,
            resources.common.repos.notifications.as_ref(),
        )
        .await;

        let duration_ms = u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);
        if response.error.is_none() {
            Self::record_mcp_tool_usage(
                resources,
                &tenant_context,
                user_id,
                tool_name,
                duration_ms,
            )
            .await;
            if tool_name == GET_ACTIVITIES {
                Self::increment_activity_counters(resources, &tenant_context, user_id, &args).await;
            }
        }

        Self::mcp_response_to_tool_response(response)
    }

    /// Adapt an [`McpResponse`] from the shared routing helpers into the wire
    /// [`ToolResponse`] the tronc dispatcher returns.
    ///
    /// A JSON-RPC error becomes an in-band error result (MCP reports tool
    /// failures via `isError`); a success result already carries the
    /// `{content, isError, structuredContent}` shape and is deserialized back.
    fn mcp_response_to_tool_response(response: McpResponse) -> ToolResponse {
        if let Some(error) = response.error {
            return ToolResponse::error(error.message);
        }
        response.result.map_or_else(
            || ToolResponse::error("Tool produced no result".to_owned()),
            |result| {
                serde_json::from_value(result.clone()).unwrap_or_else(|_| {
                    // Result is not the tool-response shape (e.g. a bare object)
                    // — surface it as a single text item rather than dropping it.
                    ToolResponse::text(result.to_string())
                })
            },
        )
    }

    /// Check if a tool is enabled for a tenant, returning an error response if disabled
    ///
    /// Tenant context is now required - tool execution without tenant isolation is not allowed.
    async fn check_tool_enabled(
        resources: &Arc<ServerContext>,
        tenant_context: &TenantContext,
        tool_name: &str,
        request_id: Option<Value>,
    ) -> Option<McpResponse> {
        match resources
            .mcp
            .tool_selection
            .is_tool_enabled(tenant_context.tenant_id, tool_name)
            .await
        {
            Ok(true) => {
                debug!(
                    "Tool {} is enabled for tenant {}",
                    tool_name, tenant_context.tenant_id
                );
                None
            }
            Ok(false) => {
                warn!(
                    "Tool {} not enabled for tenant {} - rejecting",
                    tool_name, tenant_context.tenant_id
                );
                Some(McpResponse {
                    jsonrpc: JSONRPC_VERSION.to_owned(),
                    id: request_id,
                    result: None,
                    error: Some(McpError {
                        code: ERROR_METHOD_NOT_FOUND,
                        message: format!(
                            "Tool '{tool_name}' is not available for your tenant. \
                             Contact your administrator to enable it."
                        ),
                        data: None,
                    }),
                })
            }
            Err(e) => {
                debug!(
                    "Tool {} not in catalog ({}), allowing execution",
                    tool_name, e
                );
                None
            }
        }
    }

    /// Handle tool execution directly using provided `ServerContext`
    ///
    /// Tenant context is now required for all tool executions to ensure proper
    /// tenant isolation and tool enablement policy enforcement.
    #[tracing::instrument(
        skip(request, auth_result, tenant_context, resources),
        fields(
            tool_name = Empty,
            user_id = %auth_result.user_id,
            tenant_id = %tenant_context.tenant_id,
            success = Empty,
            duration_ms = Empty,
        )
    )]
    async fn handle_tool_execution_direct(
        request: McpRequest,
        auth_result: AuthResult,
        tenant_context: TenantContext,
        resources: &Arc<ServerContext>,
    ) -> McpResponse {
        let Some(params) = request.params else {
            error!("Missing request parameters in tools/call");
            return McpResponse {
                jsonrpc: "2.0".to_owned(),
                id: request.id,
                result: None,
                error: Some(McpError {
                    code: ERROR_INVALID_PARAMS,
                    message: "Invalid params: Missing request parameters".to_owned(),
                    data: None,
                }),
            };
        };

        // Parse tool call parameters with type safety
        let tool_params = match serde_json::from_value::<json_schemas::ToolCallParams>(params) {
            Ok(p) => p,
            Err(e) => {
                error!("Failed to parse tool call parameters: {}", e);
                return McpResponse {
                    jsonrpc: "2.0".to_owned(),
                    id: request.id,
                    result: None,
                    error: Some(McpError {
                        code: ERROR_INVALID_PARAMS,
                        message: format!("Invalid tool call parameters: {e}"),
                        data: None,
                    }),
                };
            }
        };

        let tool_name = &tool_params.name;
        let args = &tool_params.arguments;
        let user_id = auth_result.user_id;

        // Record tool name in span
        tracing::Span::current().record("tool_name", tool_name.as_str());

        // Check if tool is enabled for this tenant
        if let Some(error_response) =
            Self::check_tool_enabled(resources, &tenant_context, tool_name, request.id.clone())
                .await
        {
            return error_response;
        }

        // Activity-specific quota check (separate from general tool_calls budget)
        if let Some(error_response) = Self::check_activity_quota(
            resources,
            &tenant_context,
            auth_result.user_id,
            tool_name,
            args,
            request.id.clone(),
        )
        .await
        {
            return error_response;
        }

        let start_time = Instant::now();

        info!(
            "Executing tool call: {} for user: {} using {} authentication",
            tool_name,
            user_id,
            auth_result.auth_method.display_name()
        );

        // Use the provided ServerContext directly - no fake resource creation!
        let tool_context =
            Self::build_tool_context(user_id, request.id.clone(), &tenant_context, &auth_result);
        let routing_context = ToolRoutingContext {
            resources,
            tenant_context: &tenant_context,
            tool_context: &tool_context,
        };

        let result = Self::route_tool_call(
            tool_name,
            args,
            request.id.unwrap_or_else(default_request_id),
            user_id,
            &routing_context,
        )
        .await;

        // Automatically append unread OAuth notifications to successful responses
        debug!(
            "About to check for OAuth notifications for user {} after {} tool call",
            user_id, tool_name
        );
        let result = Self::append_oauth_notifications_to_response(
            result,
            user_id,
            tool_name,
            routing_context
                .resources
                .common
                .repos
                .notifications
                .as_ref(),
        )
        .await;

        // Record completion metrics in span
        let duration = start_time.elapsed();
        let duration_ms = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        let success = result.error.is_none();

        tracing::Span::current()
            .record("duration_ms", duration_ms)
            .record("success", success);

        if success {
            info!(
                "Tool call completed successfully: {} for user: {} in {}ms",
                tool_name, user_id, duration_ms
            );

            // Fire-and-forget: increment usage counters and record llm_usage
            Self::record_mcp_tool_usage(
                resources,
                &tenant_context,
                user_id,
                tool_name,
                duration_ms,
            )
            .await;

            // Increment activity-specific counters after successful get_activities
            if tool_name == GET_ACTIVITIES {
                Self::increment_activity_counters(resources, &tenant_context, user_id, args).await;
            }
        } else {
            warn!(
                "Tool call failed: {} for user: {} in {}ms - {:?}",
                tool_name, user_id, duration_ms, result.error
            );
        }

        result
    }

    /// Handle authentication error
    fn handle_authentication_error(request: McpRequest, e: &AppError) -> McpResponse {
        warn!("MCP tool call authentication failed: {}", e);

        // Determine specific error code based on error message
        let error_message = e.to_string();
        let (error_code, error_msg) = if error_message.contains("JWT token expired") {
            (ERROR_TOKEN_EXPIRED, MSG_TOKEN_EXPIRED)
        } else if error_message.contains("JWT token signature is invalid") {
            (ERROR_TOKEN_INVALID, MSG_TOKEN_INVALID)
        } else if error_message.contains("JWT token is malformed") {
            (ERROR_TOKEN_MALFORMED, MSG_TOKEN_MALFORMED)
        } else {
            (ERROR_UNAUTHORIZED, "Authentication required")
        };

        McpResponse::error_with_data(
            request.id,
            error_code,
            error_msg.to_owned(),
            serde_json::json!({
                "detailed_error": error_message,
                "authentication_failed": true
            }),
        )
    }

    /// Increment usage counters and record `llm_usage` after a successful MCP tool call
    ///
    /// Increments `daily_tool_calls` and `weekly_tool_calls` counters (shared budget
    /// with chat route), and inserts an `llm_usage` record with `call_type = "mcp_tool"`
    /// for analytics. Errors are logged but never propagated to avoid failing the
    /// tool response.
    async fn record_mcp_tool_usage(
        resources: &Arc<ServerContext>,
        tenant_context: &TenantContext,
        user_id: Uuid,
        tool_name: &str,
        duration_ms: u64,
    ) {
        let tenant_id_str = tenant_context.tenant_id.to_string();
        let user_id_str = user_id.to_string();

        Self::increment_tool_counters(resources, &tenant_id_str, &user_id_str, tool_name).await;
        Self::insert_tool_llm_usage(
            resources,
            &tenant_id_str,
            &user_id_str,
            tool_name,
            duration_ms,
        )
        .await;
    }

    /// Increment shared daily/weekly tool call counters (fire-and-forget)
    async fn increment_tool_counters(
        resources: &Arc<ServerContext>,
        tenant_id: &str,
        user_id: &str,
        tool_name: &str,
    ) {
        // Record against tier defaults even when admin config is absent,
        // so the always-on enforcement path keeps seeing real counts.
        let admin_config: &dyn AdminConfigLookup = match resources.coach.admin_config.as_deref() {
            Some(c) => c,
            None => default_admin_config(),
        };

        let usage_svc =
            UsageCounterService::new(resources.common.repos.usage_counters.as_ref(), admin_config);
        for counter_type in &["daily_tool_calls", "weekly_tool_calls"] {
            if let Err(e) = usage_svc
                .increment(tenant_id, user_id, counter_type, 1)
                .await
            {
                warn!(
                    tool_name,
                    counter_type, "Failed to increment MCP tool counter: {e}"
                );
            }
        }

        debug!(
            tool_name,
            tenant_id, user_id, "MCP tool call counters incremented"
        );
    }

    /// Record MCP tool execution in `llm_usage` table for analytics (fire-and-forget)
    async fn insert_tool_llm_usage(
        resources: &Arc<ServerContext>,
        tenant_id: &str,
        user_id: &str,
        tool_name: &str,
        duration_ms: u64,
    ) {
        #[allow(clippy::cast_possible_wrap)]
        let exec_time_ms = duration_ms as i64;

        // A direct MCP tool invocation is its own inbound boundary — no
        // upstream chat turn exists to propagate from, so the turn id is
        // generated here and the `tools_called` list carries the single
        // tool that just executed.
        let turn_id = ConversationTurnId::new();
        let tools_called_json =
            serde_json::to_string(&[tool_name]).unwrap_or_else(|_| "[]".to_owned());

        if let Err(e) = resources
            .common
            .repos
            .llm_usage
            .insert_llm_usage(&InsertLlmUsage {
                tenant_id,
                user_id,
                conversation_id: None,
                turn_id,
                provider: "direct",
                // model column stores the tool name for mcp_tool call_type
                // (no LLM model involved in direct tool execution)
                model: tool_name,
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                cached_tokens: 0,
                call_type: "mcp_tool",
                tool_calls_count: 1,
                tools_called: &tools_called_json,
                execution_time_ms: Some(exec_time_ms),
                // Data-only MCP tool calls (Strava fetch, etc.) are not
                // LLM requests, so no USD cost applies. They still count
                // toward `daily_tool_calls` at the counter layer.
                cost_usd: 0.0,
                call_sequence: None,
            })
            .await
        {
            warn!(
                tool_name,
                "Failed to record MCP tool usage in llm_usage: {e}"
            );
        }
    }

    /// Check daily and weekly tool call quotas before executing a tool
    ///
    /// Returns `Some(McpResponse)` with a JSON-RPC error if any quota is exceeded,
    /// or `None` if execution is allowed. Uses the same `daily_tool_calls` and
    /// `weekly_tool_calls` counters as the chat route for shared budget enforcement.
    /// Admin and owner roles bypass quota enforcement entirely.
    async fn check_tool_quota(
        resources: &Arc<ServerContext>,
        tenant_context: &TenantContext,
        user_id: Uuid,
        request_id: Option<Value>,
    ) -> Option<McpResponse> {
        // Admin role bypasses quota enforcement for debugging and testing.
        // Owners (tenant creators) remain subject to quotas as cost control.
        if let Ok(Some(role)) = resources
            .common
            .repos
            .tenants
            .get_user_role(user_id, tenant_context.tenant_id)
            .await
        {
            if role == "admin" {
                debug!("Skipping MCP tool quota check for admin user {}", user_id);
                return None;
            }
        }

        let tenant_id_str = tenant_context.tenant_id.to_string();
        let user_id_str = user_id.to_string();
        let tier = Self::resolve_user_tier(resources, user_id).await;
        // Degrade to tier defaults when admin config is unavailable
        // rather than skipping enforcement.
        let admin_config: &dyn AdminConfigLookup = match resources.coach.admin_config.as_deref() {
            Some(c) => c,
            None => default_admin_config(),
        };
        let usage_svc =
            UsageCounterService::new(resources.common.repos.usage_counters.as_ref(), admin_config);

        // Check daily, then weekly quota
        for counter_type in &["daily_tool_calls", "weekly_tool_calls"] {
            if let Some(response) = Self::check_single_quota(
                &usage_svc,
                &tenant_id_str,
                &user_id_str,
                counter_type,
                &tier,
                request_id.clone(),
            )
            .await
            {
                return Some(response);
            }
        }

        None
    }

    /// Resolve the user's tier from the `users` row. Falls back to
    /// [`UserTier::Starter`] when the row cannot be loaded so the
    /// least-permissive defaults apply when the user has been deleted
    /// out from under an in-flight tool call.
    async fn resolve_user_tier(resources: &Arc<ServerContext>, user_id: Uuid) -> UserTier {
        match resources.common.repos.users.get_global(user_id).await {
            Ok(Some(user)) => user.tier,
            _ => UserTier::Starter,
        }
    }

    /// Check a single quota counter and return an error response if exceeded
    async fn check_single_quota(
        usage_svc: &UsageCounterService<'_>,
        tenant_id: &str,
        user_id: &str,
        counter_type: &str,
        tier: &UserTier,
        request_id: Option<Value>,
    ) -> Option<McpResponse> {
        match usage_svc
            .check_limit_for_tier(tenant_id, user_id, counter_type, tier)
            .await
        {
            Ok(check) if !check.allowed => {
                warn!(
                    tenant_id,
                    user_id,
                    current = check.current,
                    limit = check.limit,
                    counter_type,
                    "Tool call quota exceeded"
                );
                Some(Self::build_rate_limit_error(
                    request_id,
                    counter_type,
                    check.current,
                    check.limit,
                    &check.resets_at,
                ))
            }
            Err(e) => {
                warn!(counter_type, "Failed to check tool call quota: {e}");
                None
            }
            _ => None,
        }
    }

    /// Build a JSON-RPC error response for rate limit exceeded
    fn build_rate_limit_error(
        request_id: Option<Value>,
        limit_type: &str,
        current: i64,
        limit: i64,
        resets_at: &str,
    ) -> McpResponse {
        McpResponse::error_with_data(
            request_id,
            ERROR_RATE_LIMIT_EXCEEDED,
            "Rate limit exceeded".to_owned(),
            json!({
                "limit_type": limit_type,
                "current": current,
                "limit": limit,
                "resets_at": resets_at,
            }),
        )
    }

    /// Determine the activity mode from tool call arguments
    ///
    /// Reads the `mode` parameter from the `get_activities` args.
    /// Returns `"summary"` (default) or `"detailed"`.
    fn activity_mode_from_args(args: &Value) -> &str {
        args.get("mode")
            .and_then(Value::as_str)
            .unwrap_or("summary")
    }

    /// Check activity access quotas for `get_activities` calls
    ///
    /// Applies separate quota counters based on activity mode (summary vs detailed).
    /// Detailed mode has lower limits because it consumes more tokens (~1350 vs ~135).
    /// Returns `Some(McpResponse)` if any activity quota is exceeded, `None` if allowed.
    async fn check_activity_quota(
        resources: &Arc<ServerContext>,
        tenant_context: &TenantContext,
        user_id: Uuid,
        tool_name: &str,
        args: &Value,
        request_id: Option<Value>,
    ) -> Option<McpResponse> {
        if tool_name != GET_ACTIVITIES {
            return None;
        }

        let tenant_id_str = tenant_context.tenant_id.to_string();
        let user_id_str = user_id.to_string();
        let tier = Self::resolve_user_tier(resources, user_id).await;
        // Degrade to tier defaults when admin config is unavailable
        // rather than skipping enforcement.
        let admin_config: &dyn AdminConfigLookup = match resources.coach.admin_config.as_deref() {
            Some(c) => c,
            None => default_admin_config(),
        };
        let usage_svc =
            UsageCounterService::new(resources.common.repos.usage_counters.as_ref(), admin_config);

        let mode = Self::activity_mode_from_args(args);
        let counter_types = if mode == "detailed" {
            &["daily_activity_detailed", "weekly_activity_detailed"][..]
        } else {
            &["daily_activity_summary", "weekly_activity_summary"][..]
        };

        for counter_type in counter_types {
            if let Some(response) = Self::check_single_quota(
                &usage_svc,
                &tenant_id_str,
                &user_id_str,
                counter_type,
                &tier,
                request_id.clone(),
            )
            .await
            {
                return Some(response);
            }
        }

        None
    }

    /// Increment activity-specific counters after a successful `get_activities` call
    ///
    /// Uses the mode parameter to determine which counters to increment
    /// (summary vs detailed). Fire-and-forget: errors are logged but not propagated.
    async fn increment_activity_counters(
        resources: &Arc<ServerContext>,
        tenant_context: &TenantContext,
        user_id: Uuid,
        args: &Value,
    ) {
        // Record against tier defaults even when admin config is absent.
        let admin_config: &dyn AdminConfigLookup = match resources.coach.admin_config.as_deref() {
            Some(c) => c,
            None => default_admin_config(),
        };

        let tenant_id_str = tenant_context.tenant_id.to_string();
        let user_id_str = user_id.to_string();
        let usage_svc =
            UsageCounterService::new(resources.common.repos.usage_counters.as_ref(), admin_config);

        let mode = Self::activity_mode_from_args(args);
        let counter_types = if mode == "detailed" {
            &["daily_activity_detailed", "weekly_activity_detailed"][..]
        } else {
            &["daily_activity_summary", "weekly_activity_summary"][..]
        };

        for counter_type in counter_types {
            if let Err(e) = usage_svc
                .increment(&tenant_id_str, &user_id_str, counter_type, 1)
                .await
            {
                warn!(
                    counter_type,
                    mode, "Failed to increment activity counter: {e}"
                );
            }
        }

        debug!(
            mode,
            tenant_id = %tenant_id_str,
            user_id = %user_id_str,
            "Activity access counters incremented"
        );
    }

    /// Build the per-call [`ToolContext`] handed to the tool registry.
    ///
    /// The admin flag is resolved from the tenant context — the same signal that
    /// gates which tools `tools/list` advertises — so execution and discovery
    /// agree on what an admin caller may invoke. Used by the SSE entry path; the
    /// HTTP path's context is built by the tronc auth hook.
    fn build_tool_context(
        user_id: Uuid,
        request_id: Option<Value>,
        tenant_context: &TenantContext,
        auth_result: &AuthResult,
    ) -> ToolContext {
        // Map AuthResult auth_method to the host-agnostic label.
        let auth_method = match &auth_result.auth_method {
            AuthResultMethod::JwtToken { .. } => AuthMethod::JwtBearer,
            AuthResultMethod::ApiKey { .. } => AuthMethod::ApiKey,
            AuthResultMethod::ChannelLink { .. } => AuthMethod::ChannelLink,
        };

        let mut tool_ctx = ToolContext::new()
            .with_user(user_id.to_string())
            .with_tenant(tenant_context.tenant_id.to_string())
            .with_auth_method(auth_method.as_str())
            .as_admin(tenant_context.is_admin());

        // Add request ID if available
        if let Some(req_id) = request_id {
            tool_ctx = tool_ctx.with_request_id(req_id);
        }

        tool_ctx
    }

    /// Convert a tool's [`ToolResponse`] to an `McpResponse`.
    ///
    /// The tool registry returns the wire-shaped [`ToolResponse`] directly
    /// (`content` + `isError` + optional `structuredContent`), which is the MCP
    /// `CallToolResult` shape — including the dual text/structured representation
    /// for cross-version client interoperability. Tool failures are reported
    /// in-band via `isError`, not as protocol-level JSON-RPC errors.
    fn tool_response_to_mcp_response(response: &ToolResponse, request_id: Value) -> McpResponse {
        let result = serde_json::to_value(response).unwrap_or_else(|_| {
            json!({
                "content": [{ "type": "text", "text": "Tool result serialization failed" }],
                "isError": true,
            })
        });
        McpResponse {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id: Some(request_id),
            result: Some(result),
            error: None,
        }
    }

    /// Route tool calls to appropriate handlers based on tool type and tenant context
    ///
    /// Uses the `ToolRegistry` for tool execution. OAuth connection tools are handled
    /// specially due to their complex flow requirements.
    pub async fn route_tool_call(
        tool_name: &str,
        args: &Value,
        request_id: Value,
        user_id: Uuid,
        ctx: &ToolRoutingContext<'_>,
    ) -> McpResponse {
        // Handle OAuth connection tools specially — they have complex flow
        // requirements (minting hosted-login / provider-authorize URLs, MCP
        // request-id shaping) that don't fit the standard McpTool pattern.
        //
        // S4 CARVE-OUT: this is the ONE registered-tool path that bypasses
        // `UniversalExecutor::execute_tool`, so `disconnect_provider`'s
        // `IRREVERSIBLE` Guardian label does not fire here. connect/get_status are
        // non-destructive (safe to bypass); disconnect_provider IS destructive,
        // and its carve-out handler (`route_disconnect_tool` → provider_registry +
        // coach DB) does MORE than the registry `DisconnectProviderTool`
        // (plain `delete_token`), so it cannot simply fall through to the
        // chokepoint without a behaviour change. Closing this gap needs a design
        // decision (canonicalize disconnect on the registry tool, or gate the
        // carve-out inline) — see the guardians-review #1 follow-up.
        match tool_name {
            CONNECT_PROVIDER => {
                return Self::handle_connect_provider(args, request_id);
            }
            GET_CONNECTION_STATUS => {
                return Self::handle_get_connection_status(args, request_id, ctx).await;
            }
            DISCONNECT_PROVIDER => {
                return Self::handle_disconnect_provider(args, request_id, ctx).await;
            }
            _ => {}
        }

        // Try the registry first for all other tools
        if ctx.resources.mcp.tool_registry.contains(tool_name) {
            // Route through the unified executor so every registry tool call
            // passes the Guardian chokepoint and resolves identity/admin/tenant
            // consistently via `build_tool_context`. (Previously this called
            // `ToolRegistry::execute` directly, bypassing the executor.)
            let executor = ctx.tenant_context.session_id.clone().map_or_else(
                || UniversalToolExecutor::new(ctx.resources.clone()),
                |turn| UniversalToolExecutor::new(ctx.resources.clone()).with_turn_token(turn),
            );
            let request = UniversalRequest {
                tool_name: tool_name.to_owned(),
                parameters: args.clone(),
                user_id: user_id.to_string(),
                protocol: "mcp".to_owned(),
                tenant_id: Some(ctx.tenant_context.tenant_id.to_string()),
                progress_token: None,
                cancellation_token: None,
                progress_reporter: None,
            };
            let response = match executor.execute_tool(request).await {
                Ok(universal) => ProtocolConverter::universal_to_mcp(universal),
                Err(e) => {
                    // Preserve the machine-detectable reconnect trigger for MCP
                    // clients: a raised ProviderAuthRequired carries a structured
                    // error_code + provider so the client can reconnect, instead
                    // of the flattened prose the caller would otherwise get.
                    if let Some(provider) = e.provider_auth_required_provider() {
                        return Self::provider_auth_required_mcp(provider, request_id);
                    }
                    ToolResponse::error(format!("Tool execution failed: {e}"))
                }
            };
            Self::tool_response_to_mcp_response(&response, request_id)
        } else {
            // Fall back to provider tool routing for tools not in the registry
            ProviderToolRouter::route_provider_tool(tool_name, args, request_id, user_id, ctx).await
        }
    }

    /// Build a structured MCP error for a raised `ProviderAuthRequired`.
    ///
    /// Preserves the machine-detectable `error_code` + `provider` in `data` so an
    /// MCP client can trigger a reconnect rather than string-parse a prose
    /// failure — the structured signal the pre-guardian in-band path carried.
    fn provider_auth_required_mcp(provider: &str, request_id: Value) -> McpResponse {
        McpResponse {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            result: None,
            error: Some(McpError {
                code: ERROR_INTERNAL_ERROR,
                message: format!(
                    "Provider authentication required for '{provider}'. Reconnect the provider and retry."
                ),
                data: Some(json!({
                    "error_code": "provider_auth_required",
                    "provider": provider,
                })),
            }),
            id: Some(request_id),
        }
    }

    /// Handle `connect_provider` OAuth tool
    fn handle_connect_provider(args: &Value, request_id: Value) -> McpResponse {
        let params = serde_json::from_value::<json_schemas::ConnectProviderParams>(args.clone())
            .unwrap_or_else(|_| json_schemas::ConnectProviderParams {
                provider: String::new(),
                strava_client_id: None,
                strava_client_secret: None,
                fitbit_client_id: None,
                fitbit_client_secret: None,
            });

        let provider_name = params.provider.to_lowercase();

        // Validate provider
        if provider_name.is_empty() || !["strava", "fitbit"].contains(&provider_name.as_str()) {
            return McpResponse {
                jsonrpc: JSONRPC_VERSION.to_owned(),
                id: Some(request_id),
                result: Some(json!({
                    "content": [{
                        "type": "text",
                        "text": format!("Invalid provider '{provider_name}'. Supported providers are: strava, fitbit")
                    }],
                    "isError": true
                })),
                error: None,
            };
        }

        // Return unified auth flow response
        McpResponse {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id: Some(request_id),
            result: Some(json!({
                "content": [{
                    "type": "text",
                    "text": format!(
                        "Starting unified authentication for {}. This will:\n\n1. First authenticate you with Pierre Fitness Server\n2. Then connect you to {} for your fitness data\n\nOpening browser for secure authentication...",
                        provider_name.to_uppercase(),
                        provider_name.to_uppercase()
                    )
                }],
                "isError": false,
                "requiresAuth": true,
                "authUrl": "oauth2/authorize",
                "unifiedFlow": true,
                "provider": provider_name,
                "message": format!("Please complete unified authentication with Pierre and {} in your browser.", provider_name.to_uppercase())
            })),
            error: None,
        }
    }

    /// Handle `get_connection_status` OAuth tool
    ///
    /// Tenant context is always available since tool execution requires it.
    async fn handle_get_connection_status(
        args: &Value,
        request_id: Value,
        ctx: &ToolRoutingContext<'_>,
    ) -> McpResponse {
        let params =
            serde_json::from_value::<json_schemas::GetConnectionStatusParams>(args.clone())
                .unwrap_or_default();

        let credentials = McpOAuthCredentials {
            strava_client_id: params.strava_client_id.as_deref(),
            strava_client_secret: params.strava_client_secret.as_deref(),
            fitbit_client_id: params.fitbit_client_id.as_deref(),
            fitbit_client_secret: params.fitbit_client_secret.as_deref(),
        };

        ProviderToolRouter::handle_tenant_connection_status(
            ctx.tenant_context,
            &ctx.resources.auth.tenant_oauth_client,
            &ctx.resources.common.repos,
            request_id,
            credentials,
            ctx.resources.common.config.http_port,
            &ctx.resources.common.config,
        )
        .await
    }

    /// Handle `disconnect_provider` OAuth tool
    async fn handle_disconnect_provider(
        args: &Value,
        request_id: Value,
        ctx: &ToolRoutingContext<'_>,
    ) -> McpResponse {
        let params =
            match serde_json::from_value::<json_schemas::DisconnectProviderParams>(args.clone()) {
                Ok(p) => p,
                Err(e) => {
                    return McpResponse {
                        jsonrpc: JSONRPC_VERSION.to_owned(),
                        result: None,
                        error: Some(McpError {
                            code: ERROR_INVALID_PARAMS,
                            message: format!("Invalid disconnect_provider parameters: {e}"),
                            data: None,
                        }),
                        id: Some(request_id),
                    };
                }
            };

        ProviderToolRouter::route_disconnect_tool(&params.provider, request_id, ctx).await
    }

    /// Build notification text from a list of OAuth notifications
    fn build_notification_text(notifications: &[OAuthNotification]) -> String {
        let mut notification_text = String::from("\n\nOAuth Connection Updates:\n");
        for notification in notifications {
            let status_indicator = if notification.success {
                "[SUCCESS]"
            } else {
                "[FAILED]"
            };
            writeln!(
                &mut notification_text,
                "{} {}: {}",
                status_indicator,
                notification.provider.to_uppercase(),
                notification.message
            )
            .unwrap_or_else(|_| warn!("Failed to write notification text"));
        }
        notification_text
    }

    /// Append notification text to an MCP response result
    fn append_notification_to_result(result: &mut Value, notification_text: &str) {
        if let Some(content) = result.get_mut("content") {
            if let Some(text_value) = content.as_array_mut() {
                text_value.push(json!({
                    "type": "text",
                    "text": notification_text
                }));
                return;
            }
            if let Some(text_str) = content.as_str() {
                *content = json!(format!("{text_str}{notification_text}"));
                return;
            }
        }

        if let Some(message) = result.get_mut("message") {
            if let Some(msg_str) = message.as_str() {
                *message = json!(format!("{msg_str}{notification_text}"));
                return;
            }
        }

        if let Some(obj) = result.as_object_mut() {
            obj.insert("oauth_notifications".to_owned(), json!(notification_text));
        }
    }

    /// Mark a list of notifications as read in the database
    async fn mark_notifications_read(
        notifications_repo: &dyn NotificationRepository,
        notifications: &[OAuthNotification],
        user_id: Uuid,
    ) {
        for notification in notifications {
            if let Err(e) = notifications_repo
                .mark_read(&notification.id, user_id)
                .await
            {
                warn!(
                    "Failed to mark notification {} as read after delivery: {}",
                    notification.id, e
                );
            }
        }
    }

    /// Check if a tool name should skip notification checking
    /// Note: With OAuth notification tools removed, this always returns false.
    /// Kept for potential future tools that might need to skip notification checks.
    const fn should_skip_notification_check(_tool_name: &str) -> bool {
        false
    }

    /// Check if notifications should be fetched for this response
    fn should_fetch_notifications(response: &McpResponse, tool_name: &str, user_id: Uuid) -> bool {
        if response.error.is_some() {
            debug!(
                "NOTIFICATION_CHECK: Skipping due to error response for user {}",
                user_id
            );
            return false;
        }

        if Self::should_skip_notification_check(tool_name) {
            debug!(
                "NOTIFICATION_CHECK: Skipping for notification-related tool {} for user {}",
                tool_name, user_id
            );
            return false;
        }

        true
    }

    /// Fetch unread notifications if any exist
    async fn fetch_unread_notifications(
        notifications_repo: &dyn NotificationRepository,
        user_id: Uuid,
        tool_name: &str,
    ) -> Option<Vec<OAuthNotification>> {
        match notifications_repo.get_unread(user_id).await {
            Ok(notifications) if !notifications.is_empty() => {
                debug!(
                    "Found {} unread OAuth notifications for user {} during {} tool call",
                    notifications.len(),
                    user_id,
                    tool_name
                );
                Some(notifications)
            }
            Ok(_) => {
                debug!(
                    "NOTIFICATION_CHECK: No unread notifications found for user {} during {} tool call",
                    user_id, tool_name
                );
                None
            }
            Err(e) => {
                warn!(
                    "Failed to check OAuth notifications for user {} during {} tool call: {}",
                    user_id, tool_name, e
                );
                None
            }
        }
    }

    /// Automatically append unread OAuth notifications to successful tool responses
    async fn append_oauth_notifications_to_response(
        mut response: McpResponse,
        user_id: Uuid,
        tool_name: &str,
        notifications_repo: &dyn NotificationRepository,
    ) -> McpResponse {
        debug!(
            "NOTIFICATION_CHECK: Starting notification check for user {} with tool {}",
            user_id, tool_name
        );

        if !Self::should_fetch_notifications(&response, tool_name, user_id) {
            return response;
        }

        let Some(unread_notifications) =
            Self::fetch_unread_notifications(notifications_repo, user_id, tool_name).await
        else {
            return response;
        };

        let notification_text = Self::build_notification_text(&unread_notifications);

        if let Some(ref mut result) = response.result {
            Self::append_notification_to_result(result, &notification_text);
        }

        info!(
            "Automatically delivered {} OAuth notifications to user {} via {} tool response",
            unread_notifications.len(),
            user_id,
            tool_name
        );

        Self::mark_notifications_read(notifications_repo, &unread_notifications, user_id).await;

        response
    }
}
