// ABOUTME: Tool execution handlers for MCP server tool calls and provider routing
// ABOUTME: Runs the post-auth flow — enablement, quota, routing, usage — for an identified caller
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::multitenant::ProviderToolRouter;
use super::resources::ServerContext;
use crate::constants::{
    errors::{
        ERROR_INTERNAL_ERROR, ERROR_INVALID_PARAMS, ERROR_METHOD_NOT_FOUND,
        ERROR_RATE_LIMIT_EXCEEDED,
    },
    protocol::JSONRPC_VERSION,
    tools::{CONNECT_PROVIDER, DISCONNECT_PROVIDER, GET_ACTIVITIES},
};
use crate::mcp::audit::record_tool_call;
use dravr_tronc::mcp::schema::ToolResponse;
use dravr_tronc::mcp::tool::ToolContext;
use pierre_auth::tenant::TenantContext;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::usage::InsertLlmUsage;
use pierre_core::models::{ConversationTurnId, UserTier};
use pierre_core::models::{OAuthNotification, TenantId};
use pierre_database::backends::NotificationRepository;
use pierre_mcp_schema::json_schemas;
use pierre_mcp_schema::{McpError, McpResponse};
use pierre_mcp_transport::tenant_isolation::extract_tenant_context_internal;
use pierre_runtime_context::{default_admin_config, AdminConfigLookup};
use pierre_services::quota_policy::{check_quotas, QuotaPolicyInputs, QuotaSurface};
use pierre_services::usage_counter::{increment_counter, UsageCounterService};
use pierre_tool_runtime::guardian::{self, DenyReason, GateOutcome, TurnKey};
use pierre_tool_runtime::protocol::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::protocols::converter::ProtocolConverter;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::security::SecurityLabels;
// Other trait methods dispatched through repos.tenants / repos.llm_usage / repos.users
use serde_json::{json, Value};
use std::fmt::Write;
use std::sync::Arc;
use std::time::Instant;
use tracing::field::Empty;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Default ID for notifications and error responses that don't have a request ID
fn default_request_id() -> Value {
    Value::Number(serde_json::Number::from(0))
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
    /// Execute a tool call for an already-authenticated caller (tronc HTTP path).
    ///
    /// The tronc auth hook has resolved the caller's identity into `tool_context`
    /// (and `user_id`/`tenant_id`); this runs the post-auth flow without
    /// re-authenticating: tool-enablement, quota, activity quota, routing,
    /// OAuth-notification augmentation, and usage recording. Failures (quota,
    /// disabled tool, unknown tool) are reported in-band as an error
    /// [`ToolResponse`], per the MCP `tools/call` model.
    ///
    /// The audit fields are declared here because this is where a tool call
    /// actually arrives. `record_tool_call` used to be called one layer up, in
    /// a dispatcher no production caller reached, so the fingerprint it wrote
    /// was never recorded for a real call.
    #[tracing::instrument(
        skip(resources, _state, tool_context, args),
        fields(
            tool_name = Empty,
            arguments_hash = Empty,
            user_id = %user_id,
            tenant_id = %tenant_id,
        )
    )]
    pub async fn dispatch_tool_call(
        resources: &Arc<ServerContext>,
        _state: &Arc<dyn ToolRuntime>,
        tool_context: &ToolContext,
        user_id: Uuid,
        tenant_id: TenantId,
        tool_name: &str,
        args: Value,
    ) -> ToolResponse {
        record_tool_call(tool_name, &args);

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
            user_id,
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
        user_id: Uuid,
        tool_name: &str,
        request_id: Option<Value>,
    ) -> Option<McpResponse> {
        match resources
            .mcp
            .tool_selection
            .is_tool_enabled_for_user(tenant_context.tenant_id, user_id, tool_name)
            .await
        {
            Ok(true) => {
                debug!(
                    "Tool {} is enabled for user {} in tenant {}",
                    tool_name, user_id, tenant_context.tenant_id
                );
                None
            }
            Ok(false) => {
                warn!(
                    "Tool {} not enabled for user {} in tenant {} - rejecting",
                    tool_name, user_id, tenant_context.tenant_id
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

        // The charge for a REGISTRY tool is levied at the dispatch chokepoint
        // in `UniversalExecutor::execute_tool`, which every transport passes
        // through; charging again here billed an ACP turn twice for one tool.
        // The two OAuth carve-outs above never reach that chokepoint, so
        // their charge is still owed here — same two constants the router
        // matches on, so the two cannot drift into disagreement.
        if matches!(tool_name, CONNECT_PROVIDER | DISCONNECT_PROVIDER) {
            Self::charge_carve_out_tool_call(resources, &tenant_id_str, &user_id_str, tool_name)
                .await;
        }
        Self::insert_tool_llm_usage(
            resources,
            &tenant_id_str,
            &user_id_str,
            tool_name,
            duration_ms,
        )
        .await;
    }

    /// Charge one tool call for a carve-out tool, which bypasses the executor.
    ///
    /// Fire-and-forget: a counter write must never fail a tool the caller has
    /// already been answered by.
    async fn charge_carve_out_tool_call(
        resources: &Arc<ServerContext>,
        tenant_id: &str,
        user_id: &str,
        tool_name: &str,
    ) {
        let repo = resources.common.repos.usage_counters.as_ref();
        for counter_type in ["daily_tool_calls", "weekly_tool_calls"] {
            if let Err(e) = increment_counter(repo, tenant_id, user_id, counter_type, 1).await {
                warn!(tool_name, counter_type, "failed to charge tool call: {e}");
            }
        }
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
            .insert_llm_usage(&InsertLlmUsage::for_direct_tool_call(
                tenant_id,
                user_id,
                turn_id,
                tool_name,
                &tools_called_json,
                exec_time_ms,
            ))
            .await
        {
            warn!(
                tool_name,
                "Failed to record MCP tool usage in llm_usage: {e}"
            );
        }
    }

    /// Check the usage caps that apply to a direct `POST /mcp` tool call.
    /// Returns `Some(McpResponse)` with a JSON-RPC error if any cap is
    /// breached, or `None` if execution is allowed.
    ///
    /// The decision belongs to [`pierre_services::quota_policy::check_quotas`]
    /// with [`QuotaSurface::McpToolCall`]: the same account ladder, tier
    /// resolution, tier-default degradation and `QUOTA_BYPASS_USER_IDS`
    /// allow-list a chat turn passes — plus the `daily_tool_calls` /
    /// `weekly_tool_calls` ladder this path increments after a tool runs.
    /// A second local implementation once resolved thresholds itself and
    /// exempted the admin role, which chat never did — the same account was
    /// refused at two different doors. Only the JSON-RPC shaping is local.
    async fn check_tool_quota(
        resources: &Arc<ServerContext>,
        tenant_context: &TenantContext,
        user_id: Uuid,
        request_id: Option<Value>,
    ) -> Option<McpResponse> {
        let inputs = QuotaPolicyInputs {
            repos: resources.common.repos.as_ref(),
            admin_config: resources
                .coach
                .admin_config
                .as_deref()
                .map(|c| c as &dyn AdminConfigLookup),
        };
        match check_quotas(
            &inputs,
            tenant_context.tenant_id,
            user_id,
            &QuotaSurface::McpToolCall,
        )
        .await
        {
            Ok(_) => None,
            Err(e) if e.code == ErrorCode::QuotaExceeded => {
                warn!(
                    user_id = %user_id,
                    tenant_id = %tenant_context.tenant_id,
                    "Tool call quota exceeded: {}", e.message
                );
                Some(Self::build_rate_limit_error(request_id, &e))
            }
            Err(e) => {
                warn!("Failed to check tool call quota: {e}");
                None
            }
        }
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

    /// Check one `get_activities` mode counter and return an error response if
    /// the hard limit is breached.
    ///
    /// Scoped to the activity-mode ladder, which is a per-tool token-cost guard
    /// rather than an account cap: it is keyed on the arguments of a single
    /// tool, so it has no chat-turn counterpart and does not belong in the
    /// shared policy.
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
                    "Activity quota exceeded"
                );
                Some(Self::build_rate_limit_error(
                    request_id,
                    &AppError::quota_exceeded(
                        counter_type,
                        check.current,
                        check.limit,
                        &check.resets_at,
                    ),
                ))
            }
            Err(e) => {
                warn!(counter_type, "Failed to check activity quota: {e}");
                None
            }
            _ => None,
        }
    }

    /// Shape a [`ErrorCode::QuotaExceeded`] refusal into the JSON-RPC error the
    /// MCP transport returns.
    ///
    /// `AppError::quota_exceeded` already carries `limit_type`, `current`,
    /// `limit` and `resets_at` in `details`, so the payload is the policy's own
    /// numbers rather than a second reading of the counters.
    fn build_rate_limit_error(request_id: Option<Value>, error: &AppError) -> McpResponse {
        let data = error
            .details
            .as_deref()
            .cloned()
            .unwrap_or_else(|| json!({ "message": error.message }));
        McpResponse::error_with_data(
            request_id,
            ERROR_RATE_LIMIT_EXCEEDED,
            "Rate limit exceeded".to_owned(),
            data,
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
        // `connect_provider` is handled specially — minting a hosted-login URL
        // and shaping the MCP request id doesn't fit McpTool.
        //
        // S4 CARVE-OUT: this path bypasses `UniversalExecutor::execute_tool`.
        // `connect_provider` is non-destructive (empty Guardian labels), so
        // bypassing the chokepoint is a no-op for it. `disconnect_provider`
        // IS `IRREVERSIBLE`; its carve-out handler and the registry
        // `DisconnectProviderTool` both delegate to the same domain chokepoint
        // (`OAuthService::disconnect_provider`), so the carve-out exists only
        // for MCP response shaping and the Guardian confirm degradation (deny,
        // where the executor would park — /mcp cannot resolve a parked
        // action). It runs the SAME shared `guardian::guardian_gate` inline
        // (#1), so taint→irreversible + the per-turn destructive budget fire
        // on this /mcp path exactly as at the chokepoint.
        //
        // `get_connection_status` deliberately has NO carve-out: it routes to
        // the registry tool like every other read, so `/mcp` and the rest of
        // the product answer the same shape from one implementation.
        match tool_name {
            CONNECT_PROVIDER => {
                return Self::handle_connect_provider(args, request_id);
            }
            DISCONNECT_PROVIDER => {
                return Self::guarded_disconnect_provider(args, request_id, ctx).await;
            }
            _ => {}
        }

        // Try the registry first for all other tools
        if ctx.resources.mcp.tool_registry.contains(tool_name) {
            // Route through the unified executor so every registry tool call
            // passes the Guardian chokepoint and resolves identity/admin/tenant
            // consistently via `build_tool_context`.
            // Guardian turn key only — a `/mcp` caller has no chat turn.
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
                        "Starting unified authentication for {}. This will:\n\n1. First authenticate you with Dravr\n2. Then connect you to {} for your fitness data\n\nOpening browser for secure authentication...",
                        provider_name.to_uppercase(),
                        provider_name.to_uppercase()
                    )
                }],
                "isError": false,
                "requiresAuth": true,
                "authUrl": "oauth2/authorize",
                "unifiedFlow": true,
                "provider": provider_name,
                "message": format!("Please complete unified authentication with Dravr and {} in your browser.", provider_name.to_uppercase())
            })),
            error: None,
        }
    }

    /// Gate `disconnect_provider` through the shared Guardian decision before the
    /// carve-out handler runs (#1).
    ///
    /// `disconnect_provider` is dispatched here instead of through
    /// `execute_tool`, so without this it would skip the taint→irreversible +
    /// per-turn destructive budget gate every chokepoint dispatch enforces.
    /// Labels are `IRREVERSIBLE` + `WRITES_DATA`, matching
    /// `declare_security!(DisconnectProviderTool => IRREVERSIBLE)` and its
    /// capabilities (pinned by `disconnect_provider_gate_labels_match_registry`);
    /// the reserved budget is refunded if the disconnect itself fails, mirroring
    /// the chokepoint's post-execution refund.
    async fn guarded_disconnect_provider(
        args: &Value,
        request_id: Value,
        ctx: &ToolRoutingContext<'_>,
    ) -> McpResponse {
        let labels = SecurityLabels::IRREVERSIBLE;
        let writes_data = true;
        let tenant_uuid = Uuid::parse_str(&ctx.tenant_context.tenant_id.to_string()).ok();
        // Same turn key the chokepoint uses: the per-turn/session token so taint
        // and budgets accumulate across ONE turn's calls (a fresh nonce for a
        // token-less call makes it its own bucket).
        let turn_token = ctx
            .tenant_context
            .session_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let turn_key = TurnKey::new(tenant_uuid, turn_token);
        let (outcome, reserved) = guardian::guardian_gate(
            &ctx.resources.guardian(),
            ctx.resources.guardian_turns(),
            &turn_key,
            DISCONNECT_PROVIDER,
            labels,
            writes_data,
            tenant_uuid,
        );
        // Exhaustive on purpose: an `if let Blocked` here let `ConfirmRequired`
        // fall through and execute the disconnect. One-shot MCP-direct calls each
        // get their own turn bucket so they never reach that decision, but the
        // Copilot-headless `/mcp` loopback threads a real ACP turn token — taint
        // accumulates there, so a parked disconnect would have run. Matching every
        // variant makes the compiler catch the next one.
        match outcome {
            GateOutcome::Proceed => {}
            GateOutcome::Blocked(reason) => {
                return Self::guardian_denied_mcp(reason, request_id);
            }
            // `/mcp` is a protocol transport with no in-band way to ask a human
            // and no slash commands to resolve a parked action, so Confirm takes
            // its documented degradation — deny where no human is present. The
            // chat surfaces park and prompt instead.
            GateOutcome::ConfirmRequired => {
                return Self::guardian_denied_mcp(DenyReason::TaintedSink, request_id);
            }
        }
        let response = Self::handle_disconnect_provider(args, request_id, ctx).await;
        if reserved && Self::mcp_response_is_error(&response) {
            ctx.resources
                .guardian_turns()
                .refund(&turn_key, labels, writes_data);
        }
        response
    }

    /// Build the MCP-direct response for a Guardian-blocked carve-out tool.
    ///
    /// Shaped like a normal tool `isError` result (via
    /// [`Self::tool_response_to_mcp_response`]) carrying the machine `guardian_reason`
    /// so an MCP client sees a policy block, not a tool crash. (The chat transport
    /// localizes via `KEY_GUARDIAN_DENIED`; a raw MCP client gets the code.)
    fn guardian_denied_mcp(reason: DenyReason, request_id: Value) -> McpResponse {
        let response = ToolResponse::error(format!(
            "Blocked by the Guardian safety policy ({}).",
            reason.as_str()
        ));
        Self::tool_response_to_mcp_response(&response, request_id)
    }

    /// Whether an `McpResponse` represents a failed call (JSON-RPC error or an
    /// `isError` tool result) — used to decide whether to refund reserved budget.
    fn mcp_response_is_error(response: &McpResponse) -> bool {
        response.error.is_some()
            || response
                .result
                .as_ref()
                .and_then(|r| r.get("isError"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
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
