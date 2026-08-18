// ABOUTME: Host-integration seams wiring the platform onto the generic dravr-tronc MCP engine
// ABOUTME: AuthHook (single auth), ToolDispatcher (per-tenant list + gated call), MethodHandler (resources/prompts/completion/roots/sampling)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The platform's three host seams for the shared `dravr_tronc::mcp` engine.
//!
//! The generic [`McpServer`] natively serves `initialize`, `server/discover`,
//! `ping`, `tools/list`, and `tools/call`; the platform installs three seams to
//! supply its multi-tenant behavior:
//!
//! - [`PierreAuthHook`] — single source of authentication. The HTTP transport
//!   calls it once per request; success yields the per-call [`ToolContext`]
//!   (user/tenant/admin), failure renders as 401 (missing/invalid bearer) or
//!   403 (no tenant membership).
//! - [`PierreToolDispatcher`] — owns `tools/list` (tenant-filtered visibility)
//!   and `tools/call` (tool-enablement, quota, execution, OAuth-notification
//!   augmentation, usage recording) against the rich
//!   [`pierre_tool_runtime::registry::ToolRegistry`].
//! - [`PierreMethodHandler`] — serves the protocol areas the engine leaves open:
//!   `resources/*`, `prompts/*`, `completion/*`, `roots/*`, `sampling/*`,
//!   `authenticate`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use dravr_tronc::mcp::auth::{AuthError, AuthHook};
use dravr_tronc::mcp::host::{MethodHandler, ToolDispatcher};
use dravr_tronc::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};
use dravr_tronc::mcp::schema::{
    AuthCapability, CompleteRequest, CompleteResult, Completion, CompletionCapability,
    CreateMessageRequest, LoggingCapability, OAuth2Capability, PromptsCapability,
    ResourcesCapability, Root, SamplingCapability, ServerCapabilities, Tool, ToolResponse,
    ToolSchema, ToolsCapability,
};
use dravr_tronc::mcp::server::McpServer;
use dravr_tronc::mcp::tool::{ToolCapabilities, ToolContext, ToolRegistry};
use pierre_auth::auth::AuthResult;
use pierre_core::models::TenantId;
use pierre_mcp_schema::McpResponse;
use pierre_mcp_transport::tenant_isolation::extract_tenant_context_internal;
use pierre_tool_runtime::context::AuthMethod;
use pierre_tool_runtime::implementations::guided_flow::guided_flow_is_active;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::tool_execution::is_withheld_during_guided_flow;
use serde_json::Value;
use tracing::{debug, error, warn};
use uuid::Uuid;

use super::prompt_templates::{self, PromptGetOutcome};
use super::resource_catalog;
use super::resources::ServerContext;
use super::tool_handlers::ToolHandlers;
use crate::constants::errors::{
    ERROR_INTERNAL_ERROR, ERROR_INVALID_PARAMS, ERROR_METHOD_NOT_FOUND, ERROR_SERIALIZATION,
    MSG_SERIALIZATION,
};
use crate::constants::get_server_config;
use crate::constants::protocol::{server_name_multitenant, SERVER_VERSION};

/// Supported MCP protocol revisions advertised by the platform server, in
/// preference order.
pub const SUPPORTED_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2024-11-05"];

/// Build the per-call [`ToolContext`] the engine threads into the dispatcher.
///
/// Maps the platform's [`AuthResult`] auth method onto the host-agnostic label,
/// records user/tenant/admin, and carries the request id for correlation.
fn build_context(
    user_id: Uuid,
    tenant_id: TenantId,
    is_admin: bool,
    auth_method: &AuthResult,
    request_id: Option<Value>,
) -> ToolContext {
    use pierre_auth::auth::AuthMethod as AuthResultMethod;

    let method = match &auth_method.auth_method {
        AuthResultMethod::JwtToken { .. } => AuthMethod::JwtBearer,
        AuthResultMethod::ApiKey { .. } => AuthMethod::ApiKey,
        AuthResultMethod::ChannelLink { .. } => AuthMethod::ChannelLink,
    };

    let mut ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant_id.to_string())
        .with_auth_method(method.as_str())
        .as_admin(is_admin);

    if let Some(req_id) = request_id {
        ctx = ctx.with_request_id(req_id);
    }

    ctx
}

/// Convert a typed [`ToolSchema`] into the engine's raw-JSON [`Tool`] definition.
fn schema_to_tool(schema: ToolSchema) -> Tool {
    Tool {
        name: schema.name,
        description: schema.description,
        input_schema: serde_json::to_value(schema.input_schema).unwrap_or_default(),
        annotations: schema.annotations,
    }
}

/// Build the RFC 9728 `WWW-Authenticate` challenge value pointing clients at the
/// protected resource metadata document. `error` carries an optional RFC 6750
/// error code (e.g. `invalid_token`).
fn www_authenticate_challenge(base_url: &str, error: Option<&str>) -> String {
    let metadata_url = format!("{base_url}/.well-known/oauth-protected-resource");
    error.map_or_else(
        || format!("Bearer resource_metadata=\"{metadata_url}\""),
        |err| format!("Bearer resource_metadata=\"{metadata_url}\", error=\"{err}\""),
    )
}

/// Single-source authentication for the MCP HTTP transport.
///
/// The transport injects the HTTP `Authorization` bearer into
/// [`JsonRpcRequest::auth_token`]; this hook validates it once and resolves the
/// per-call [`ToolContext`]. A missing or invalid bearer is a 401 with the
/// RFC 9728 challenge; an authenticated caller without tenant membership is a 403.
pub struct PierreAuthHook {
    /// Shared server resources for auth + tenant resolution.
    pub resources: Arc<ServerContext>,
}

#[async_trait]
impl AuthHook<dyn ToolRuntime> for PierreAuthHook {
    async fn authenticate(
        &self,
        request: &JsonRpcRequest,
        _state: &Arc<dyn ToolRuntime>,
    ) -> Result<ToolContext, AuthError> {
        let base_url = &self.resources.common.config.base_url;

        let Some(token) = request.auth_token.as_deref() else {
            return Err(AuthError::Unauthorized {
                www_authenticate: www_authenticate_challenge(base_url, None),
            });
        };

        // The transport strips the `Bearer ` prefix; the auth middleware expects
        // the HTTP header form — `Bearer <jwt>` for JWTs, or a bare `pk_live_<key>`
        // for API keys — so reconstruct it from the stripped token.
        let auth_header = if token.starts_with("pk_live_") {
            token.to_owned()
        } else {
            format!("Bearer {token}")
        };

        let auth_result = match self
            .resources
            .auth
            .auth_middleware
            .authenticate_request(Some(&auth_header))
            .await
        {
            Ok(result) => result,
            Err(e) => {
                debug!(error = %e, "MCP request rejected: bearer token failed validation");
                return Err(AuthError::Unauthorized {
                    www_authenticate: www_authenticate_challenge(base_url, Some("invalid_token")),
                });
            }
        };

        match extract_tenant_context_internal(
            &self.resources.common.repos,
            Some(auth_result.user_id),
            auth_result.active_tenant_id.map(TenantId::from_uuid),
            None,
        )
        .await
        {
            Ok(Some(tenant_ctx)) => {
                // System-admin tools (e.g. system-coach management) gate on the
                // global `User.is_admin` flag, not the per-tenant role: a tenant
                // owner is admin *of their tenant*, which must not grant
                // system-wide admin powers. Mirrors the executor's
                // `build_tool_context`. A global-lookup failure defaults to
                // non-admin (deny-by-default).
                let is_admin = self
                    .resources
                    .common
                    .repos
                    .users
                    .get_global(auth_result.user_id)
                    .await
                    .ok()
                    .flatten()
                    .is_some_and(|user| user.is_admin);

                Ok(build_context(
                    auth_result.user_id,
                    tenant_ctx.tenant_id,
                    is_admin,
                    &auth_result,
                    request.id.clone(),
                ))
            }
            Ok(None) => Err(AuthError::Forbidden {
                reason: "User must be assigned to a tenant to execute tools".to_owned(),
            }),
            Err(e) => {
                error!(
                    user_id = %auth_result.user_id,
                    error = %e,
                    "Tenant context extraction failed - rejecting request"
                );
                Err(AuthError::Forbidden {
                    reason: "Failed to extract tenant context".to_owned(),
                })
            }
        }
    }
}

/// Host-owned tool surface backed by the rich
/// [`pierre_tool_runtime::registry::ToolRegistry`].
///
/// `list_tools` filters by the caller's tenant/admin state; `call_tool` runs the
/// full post-auth flow (enablement, quota, execution, OAuth-notification
/// augmentation, usage recording). The caller is already authenticated by
/// [`PierreAuthHook`]; the context carries the resolved identity.
pub struct PierreToolDispatcher {
    /// Shared server resources owning the tool registry + selection service.
    pub resources: Arc<ServerContext>,
}

impl PierreToolDispatcher {
    /// Resolve the tenant-filtered schemas for a non-admin caller.
    ///
    /// Combines the `ToolSelectionService` catalog (enabled, non-admin tools)
    /// with feature-flag tools not tracked by the catalog (coaches, mobility).
    /// When `user_id` is present, per-user tool overrides are overlaid on top of
    /// the tenant computation so a tool disabled for this user is hidden from
    /// discovery (and a user-enabled tool becomes visible).
    async fn tenant_filtered_schemas(
        &self,
        tenant_id: TenantId,
        user_id: Option<Uuid>,
    ) -> Vec<ToolSchema> {
        let effective = match user_id {
            Some(uid) => {
                self.resources
                    .mcp
                    .tool_selection
                    .get_effective_tools_for_user(tenant_id, uid)
                    .await
            }
            None => {
                self.resources
                    .mcp
                    .tool_selection
                    .get_effective_tools(tenant_id)
                    .await
            }
        };
        match effective {
            Ok(all_effective_tools) => {
                let enabled_names: Vec<String> = all_effective_tools
                    .iter()
                    .filter(|t| t.is_enabled)
                    .map(|t| t.tool_name.clone())
                    .collect();
                let all_catalogued_names: Vec<String> = all_effective_tools
                    .into_iter()
                    .map(|t| t.tool_name)
                    .collect();

                let mut schemas = self
                    .resources
                    .mcp
                    .tool_registry
                    .list_schemas_by_name_set(&enabled_names);
                schemas.retain(|s| {
                    self.resources
                        .mcp
                        .tool_registry
                        .get(&s.name)
                        .is_none_or(|tool| {
                            !tool.capabilities().contains(ToolCapabilities::ADMIN_ONLY)
                        })
                });

                let uncatalogued = self
                    .resources
                    .mcp
                    .tool_registry
                    .uncatalogued_user_schemas(&all_catalogued_names);
                schemas.extend(uncatalogued);

                schemas
            }
            Err(e) => {
                warn!(
                    "tools/list: failed to get tenant tools for {}: {}, falling back to user-visible",
                    tenant_id, e
                );
                self.resources.mcp.tool_registry.user_visible_schemas()
            }
        }
    }
}

#[async_trait]
impl ToolDispatcher<dyn ToolRuntime> for PierreToolDispatcher {
    async fn list_tools(&self, _state: &Arc<dyn ToolRuntime>, ctx: &ToolContext) -> Vec<Tool> {
        let mut schemas = if ctx.is_admin {
            self.resources.mcp.tool_registry.all_schemas()
        } else if let Some(tenant_id) = ctx.tenant_id.as_deref().and_then(parse_tenant_id) {
            let user_id = ctx.user_id.as_deref().and_then(|s| Uuid::parse_str(s).ok());
            self.tenant_filtered_schemas(tenant_id, user_id).await
        } else {
            self.resources.mcp.tool_registry.user_visible_schemas()
        };

        // A guided flow withholds plan-writing for its duration, and this is the
        // surface the model actually reads on the native ACP path — tool
        // visibility there comes from `tools/list`, not from `build_mcp_tools`.
        // Filtering only the latter was correct while
        // COPILOT_HEADLESS_MCP_TOOL_CALLING was false and became a silent no-op
        // the moment it was re-enabled, which is exactly the drift the shared
        // `guided_flow_is_active` predicate exists to prevent: discovery and
        // execution now answer the question the same way instead of one quietly
        // covering for the other.
        //
        // Fails closed. An unreadable onboarding state withholds rather than
        // advertises: showing the tool during a walk is what derails it, and the
        // server-side refusal would reject the call anyway.
        if let Some(tenant_id) = ctx.tenant_id.as_deref().and_then(parse_tenant_id) {
            if let Some(user_id) = ctx.user_id.as_deref() {
                let active =
                    guided_flow_is_active(&self.resources.common.repos, None, tenant_id, user_id)
                        .await
                        .unwrap_or(true);
                if active {
                    schemas.retain(|s| !is_withheld_during_guided_flow(&s.name));
                }
            }
        }

        schemas.into_iter().map(schema_to_tool).collect()
    }

    async fn call_tool(
        &self,
        name: &str,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        arguments: Value,
    ) -> ToolResponse {
        let Some(user_id) = ctx.user_id.as_deref().and_then(|s| Uuid::parse_str(s).ok()) else {
            return ToolResponse::error("Missing or invalid user identity in context".to_owned());
        };
        let Some(tenant_id) = ctx.tenant_id.as_deref().and_then(parse_tenant_id) else {
            return ToolResponse::error("Missing or invalid tenant identity in context".to_owned());
        };

        ToolHandlers::dispatch_tool_call(
            &self.resources,
            state,
            ctx,
            user_id,
            tenant_id,
            name,
            arguments,
        )
        .await
    }
}

/// Parse a tenant id string into a [`TenantId`], or `None` when malformed.
fn parse_tenant_id(value: &str) -> Option<TenantId> {
    Uuid::parse_str(value).ok().map(TenantId::from_uuid)
}

/// Handler for the JSON-RPC methods the generic engine leaves open.
///
/// Serves `resources/*`, `prompts/*`, `completion/*`, `roots/*`, `sampling/*`,
/// and `authenticate`; `initialize`/`ping`/`tools/*` are served by the engine.
pub struct PierreMethodHandler {
    /// Shared server resources backing resources/prompts/sampling.
    pub resources: Arc<ServerContext>,
}

#[async_trait]
impl MethodHandler<dyn ToolRuntime> for PierreMethodHandler {
    async fn handle(
        &self,
        method: &str,
        id: Option<Value>,
        params: Option<Value>,
        _state: &Arc<dyn ToolRuntime>,
        _ctx: &ToolContext,
    ) -> Option<JsonRpcResponse> {
        match method {
            "resources/list" => Some(self.handle_resources_list(id).await),
            "resources/read" => Some(self.handle_resources_read(id, params.as_ref()).await),
            "prompts/list" => Some(JsonRpcResponse::success(
                id,
                prompt_templates::list_prompts(),
            )),
            "prompts/get" => Some(Self::handle_prompts_get(id, params.as_ref())),
            "completion/complete" => Some(handle_completion_complete(id, params.as_ref())),
            "roots/list" => Some(handle_roots_list(id)),
            "sampling/createMessage" => Some(self.handle_create_message(id, params.as_ref()).await),
            "authenticate" => Some(handle_authenticate(id)),
            _ => None,
        }
    }
}

impl PierreMethodHandler {
    /// Serve `resources/list` — the global, publicly-published coach catalog.
    async fn handle_resources_list(&self, id: Option<Value>) -> JsonRpcResponse {
        debug!("Handling resources/list request");

        match self
            .resources
            .store_listings_repository()
            .get_published_coaches(None, None, Some(resource_catalog::list_limit()), Some(0))
            .await
        {
            Ok(published) => {
                JsonRpcResponse::success(id, resource_catalog::list_resources(&published))
            }
            Err(e) => {
                error!("resources/list: failed to load published coaches: {e}");
                JsonRpcResponse::error(id, ERROR_INTERNAL_ERROR, "Failed to load coach catalog")
            }
        }
    }

    /// Serve `resources/read` — a single published coach by `dravr://coaches/{id}` URI.
    async fn handle_resources_read(
        &self,
        id: Option<Value>,
        params: Option<&Value>,
    ) -> JsonRpcResponse {
        debug!("Handling resources/read request");

        let Some(uri) = params
            .and_then(|p| p.get("uri"))
            .and_then(serde_json::Value::as_str)
        else {
            return JsonRpcResponse::error(id, ERROR_INVALID_PARAMS, "Missing 'uri' parameter");
        };

        let Some(coach_id) = resource_catalog::coach_id_from_uri(uri) else {
            return JsonRpcResponse::error(
                id,
                ERROR_INVALID_PARAMS,
                "Unsupported resource URI scheme",
            );
        };

        let published = match self
            .resources
            .store_listings_repository()
            .get_published_coach(coach_id)
            .await
        {
            Ok(Some(coach_with_listing)) => coach_with_listing,
            Ok(None) => {
                return JsonRpcResponse::error(id, ERROR_INVALID_PARAMS, "Resource not found");
            }
            Err(e) => {
                error!("resources/read: failed to load coach: {e}");
                return JsonRpcResponse::error(id, ERROR_INTERNAL_ERROR, "Failed to load coach");
            }
        };

        match resource_catalog::read_resource(&published.coach) {
            Ok(result) => JsonRpcResponse::success(id, result),
            Err(e) => {
                error!("resources/read: failed to render coach markdown: {e}");
                JsonRpcResponse::error(id, ERROR_INTERNAL_ERROR, "Failed to render resource")
            }
        }
    }

    /// Serve `prompts/get` — assemble a curated prompt template's messages.
    fn handle_prompts_get(id: Option<Value>, params: Option<&Value>) -> JsonRpcResponse {
        debug!("Handling prompts/get request");

        let Some(name) = params
            .and_then(|p| p.get("name"))
            .and_then(serde_json::Value::as_str)
        else {
            return JsonRpcResponse::error(id, ERROR_INVALID_PARAMS, "Missing 'name' parameter");
        };

        let arguments = extract_prompt_arguments(params);

        match prompt_templates::get_prompt(name, &arguments) {
            PromptGetOutcome::Found(result) => JsonRpcResponse::success(id, result),
            PromptGetOutcome::UnknownPrompt(prompt) => JsonRpcResponse::error(
                id,
                ERROR_INVALID_PARAMS,
                format!("Unknown prompt: {prompt}"),
            ),
            PromptGetOutcome::MissingArgument { prompt, argument } => JsonRpcResponse::error(
                id,
                ERROR_INVALID_PARAMS,
                format!("Prompt '{prompt}' requires argument '{argument}'"),
            ),
        }
    }

    /// Serve `sampling/createMessage` — server-initiated LLM call over the
    /// stdio sampling peer (unavailable on HTTP transport).
    async fn handle_create_message(
        &self,
        id: Option<Value>,
        params: Option<&Value>,
    ) -> JsonRpcResponse {
        let Some(sampling_peer) = &self.resources.sse.sampling_peer else {
            return JsonRpcResponse::error(
                id,
                ERROR_METHOD_NOT_FOUND,
                "Sampling not available (stdio transport only)",
            );
        };

        let Some(params) = params else {
            return JsonRpcResponse::error(id, ERROR_INVALID_PARAMS, "Missing sampling parameters");
        };

        let create_message_request =
            match serde_json::from_value::<CreateMessageRequest>(params.clone()) {
                Ok(req) => req,
                Err(e) => {
                    error!("Failed to parse sampling parameters: {e}");
                    return JsonRpcResponse::error(
                        id,
                        ERROR_INVALID_PARAMS,
                        "Invalid sampling parameters",
                    );
                }
            };

        match sampling_peer.create_message(create_message_request).await {
            Ok(result) => match serde_json::to_value(&result) {
                Ok(result_value) => JsonRpcResponse::success(id, result_value),
                Err(e) => {
                    error!("Failed to serialize sampling result: {e}");
                    JsonRpcResponse::error(
                        id,
                        ERROR_INTERNAL_ERROR,
                        "Failed to serialize sampling result",
                    )
                }
            },
            Err(e) => {
                error!("Sampling request failed: {e}");
                JsonRpcResponse::error(id, ERROR_INTERNAL_ERROR, "Sampling request failed")
            }
        }
    }
}

/// Extract the `arguments` object from a prompts/get request into a string map.
///
/// Non-string argument values are stringified so templates always receive a
/// usable value; missing or malformed `arguments` yields an empty map.
fn extract_prompt_arguments(params: Option<&Value>) -> HashMap<String, String> {
    params
        .and_then(|p| p.get("arguments"))
        .and_then(serde_json::Value::as_object)
        .map(|map| {
            map.iter()
                .map(|(key, value)| {
                    let text = value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_owned);
                    (key.clone(), text)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Handle the `completion/complete` request for auto-complete suggestions.
///
/// Pure over the request params (no server state), so test harnesses can call it
/// directly to validate completion behavior.
#[must_use]
pub fn handle_completion_complete(id: Option<Value>, params: Option<&Value>) -> McpResponse {
    let request_id = id.unwrap_or_else(|| serde_json::json!(0));

    let complete_request: Result<CompleteRequest, _> = params
        .ok_or("Missing parameters")
        .and_then(|p| serde_json::from_value(p.clone()).map_err(|_| "Invalid parameters"));

    match complete_request {
        Ok(req) => {
            let completion = generate_completions(&req);
            let result = CompleteResult { completion };

            match serde_json::to_value(&result) {
                Ok(result_value) => McpResponse::success(Some(request_id), result_value),
                Err(e) => {
                    error!("Failed to serialize completion result: {}", e);
                    McpResponse::error(
                        Some(request_id),
                        ERROR_SERIALIZATION,
                        format!("{MSG_SERIALIZATION}: {e}"),
                    )
                }
            }
        }
        Err(e) => McpResponse::error(
            Some(request_id),
            ERROR_INVALID_PARAMS,
            format!("Invalid completion request: {e}"),
        ),
    }
}

/// Generate completion suggestions based on the request reference + argument.
fn generate_completions(req: &CompleteRequest) -> Completion {
    match req.ref_.type_.as_str() {
        "ref/prompt" => {
            if req.argument.name == "activity_type" {
                return prefix_completion(
                    &["run", "ride", "swim", "strength", "walk", "hike"],
                    &req.argument.value,
                );
            }
            if req.argument.name == "provider" {
                return prefix_completion(
                    &["strava", "fitbit", "garmin", "whoop", "terra", "sciotte"],
                    &req.argument.value,
                );
            }
            if req.argument.name == "goal_type" {
                return prefix_completion(
                    &["distance", "time", "frequency", "performance", "custom"],
                    &req.argument.value,
                );
            }
        }
        "ref/resource" => {
            return prefix_completion(&["oauth://notifications"], &req.argument.value);
        }
        _ => {}
    }

    Completion {
        values: vec![],
        total: Some(0),
        has_more: Some(false),
    }
}

/// Build a [`Completion`] from the candidates whose value starts with `prefix`.
fn prefix_completion(candidates: &[&str], prefix: &str) -> Completion {
    let matching: Vec<String> = candidates
        .iter()
        .filter(|c| c.starts_with(prefix))
        .map(|c| (*c).to_owned())
        .collect();
    Completion {
        total: Some(matching.len()),
        values: matching,
        has_more: Some(false),
    }
}

/// Handle `roots/list` — Dravr exposes no filesystem roots to MCP clients.
#[must_use]
pub fn handle_roots_list(id: Option<Value>) -> McpResponse {
    let request_id = id.unwrap_or_else(|| serde_json::json!(0));
    let roots: Vec<Root> = vec![];
    McpResponse::success(Some(request_id), serde_json::json!({ "roots": roots }))
}

/// Handle the `authenticate` method — always an invalid-params error; clients
/// authenticate via the HTTP bearer token, not this JSON-RPC method.
#[must_use]
pub fn handle_authenticate(id: Option<Value>) -> McpResponse {
    debug!("Handling authenticate request");
    McpResponse::error(
        id,
        ERROR_INVALID_PARAMS,
        "Invalid authentication parameters",
    )
}

/// Build the platform's rich [`ServerCapabilities`] for `initialize`/`discover`.
///
/// Advertises logging, prompts, resources, tools, completion, sampling, and the
/// tenant-level `OAuth2` endpoints (resolved from the configured base URL).
fn server_capabilities() -> ServerCapabilities {
    let base_url = get_server_config().map_or_else(
        || "http://localhost:8081".to_owned(),
        |c| c.base_url.clone(),
    );
    let oauth2 = OAuth2Capability {
        discovery_url: format!("{base_url}/.well-known/oauth-authorization-server"),
        authorization_endpoint: format!("{base_url}/oauth2/authorize"),
        token_endpoint: format!("{base_url}/oauth2/token"),
        registration_endpoint: format!("{base_url}/oauth2/register"),
    };

    ServerCapabilities {
        experimental: None,
        logging: Some(LoggingCapability {}),
        prompts: Some(PromptsCapability {
            list_changed: Some(false),
        }),
        resources: Some(ResourcesCapability {
            subscribe: Some(false),
            list_changed: Some(false),
        }),
        tools: Some(ToolsCapability {
            list_changed: Some(false),
        }),
        auth: Some(AuthCapability {
            oauth2: Some(oauth2.clone()),
        }),
        oauth2: Some(oauth2),
        completion: Some(CompletionCapability {}),
        sampling: Some(SamplingCapability {}),
    }
}

/// Natural-language instructions advertised to MCP clients in `initialize`.
const SERVER_INSTRUCTIONS: &str = "This server provides fitness data tools for Strava and Fitbit integration. OAuth must be configured at tenant level via REST API. Use `get_activities`, `get_athlete`, and other analytics tools to access your fitness data.";

/// Build the platform's [`McpServer`] over the shared [`ToolRuntime`] façade.
///
/// The rich tool registry, per-tenant filtering, quota, and protocol areas are
/// supplied through the three host seams; the engine's own (empty) registry is
/// unused. Reused by both the HTTP route layer and the stdio binary entry point.
///
/// The `Origin` allowlist comes from `MCP_ALLOWED_ORIGINS`
/// ([`McpConfig::allowed_origins`]) rather than the CORS list, which is
/// wildcarded in deployed environments; the HTTP transport rejects a
/// present-but-unlisted origin with 403 before authentication.
///
/// [`McpConfig::allowed_origins`]: pierre_config::mcp::McpConfig::allowed_origins
#[must_use]
pub fn build_mcp_server(resources: Arc<ServerContext>) -> Arc<McpServer<dyn ToolRuntime>> {
    let state: Arc<dyn ToolRuntime> = resources.clone();
    let supported: Vec<String> = SUPPORTED_VERSIONS.iter().map(|v| (*v).to_owned()).collect();
    let allowed_origins = resources.common.config.mcp.allowed_origins.clone();

    let server = McpServer::new(
        server_name_multitenant(),
        SERVER_VERSION,
        ToolRegistry::new(),
        state,
    )
    .with_capabilities(server_capabilities())
    .with_instructions(SERVER_INSTRUCTIONS)
    .with_supported_versions(supported)
    .with_allowed_origins(allowed_origins)
    .with_auth_hook(Arc::new(PierreAuthHook {
        resources: resources.clone(),
    }))
    .with_tool_dispatcher(Arc::new(PierreToolDispatcher {
        resources: resources.clone(),
    }))
    .with_method_handler(Arc::new(PierreMethodHandler { resources }));

    Arc::new(server)
}
