// ABOUTME: MCP request processing and protocol handling for multi-tenant server
// ABOUTME: Validates, routes, and executes MCP protocol requests with proper error handling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//
// NOTE: All `.clone()` calls in this file are Safe - they are necessary for:
// - Request/response ownership transfers across async boundaries
// - Resource Arc sharing for concurrent request processing
// - JSON value ownership for MCP protocol serialization

use super::prompt_templates::{self, PromptGetOutcome};
use super::resource_catalog;
use super::{resources::ServerContext, tool_handlers::ToolHandlers};
use crate::constants::errors::{
    ERROR_INTERNAL_ERROR, ERROR_INVALID_PARAMS, ERROR_METHOD_NOT_FOUND, ERROR_SERIALIZATION,
    ERROR_VERSION_MISMATCH, MSG_SERIALIZATION, MSG_VERSION_MISMATCH,
};
use crate::constants::get_server_config;
use crate::constants::protocol::{server_name_multitenant, JSONRPC_VERSION, SERVER_VERSION};
use crate::constants::tools::PUBLIC_DISCOVERY_TOOLS;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_mcp_schema::{
    CompleteRequest, CompleteResult, Completion, CreateMessageRequest, InitializeRequest,
    InitializeResponse, Root, ToolSchema,
};
use pierre_mcp_schema::{McpError, McpRequest, McpResponse};
use pierre_mcp_transport::tenant_isolation::extract_tenant_context_internal;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncWriteExt, Stdout};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

/// Processes MCP protocol requests with validation, routing, and execution
pub struct McpRequestProcessor {
    resources: Arc<ServerContext>,
}

impl McpRequestProcessor {
    /// Create a new MCP request processor
    #[must_use]
    pub const fn new(resources: Arc<ServerContext>) -> Self {
        Self { resources }
    }

    /// Handle an MCP request and return a response
    pub async fn handle_request(&self, request: McpRequest) -> Option<McpResponse> {
        let start_time = Instant::now();

        // Log request with optional truncation
        Self::log_request(&request);

        // Handle notifications (no response needed)
        if request.method.starts_with("notifications/") {
            self.handle_notification(&request).await;
            Self::log_completion("notification", start_time);
            return None;
        }

        // Process request and generate response
        let response = self.process_or_error(request).await;

        Self::log_completion("request", start_time);
        Some(response)
    }

    async fn process_or_error(&self, request: McpRequest) -> McpResponse {
        match self.process_request(request.clone()).await {
            Ok(response) => response,
            Err(e) => Self::create_error_response(&request, &e),
        }
    }

    fn create_error_response(request: &McpRequest, e: &AppError) -> McpResponse {
        error!(
            "Failed to process MCP request: {} | Request: method={}, jsonrpc={}, id={:?}",
            e, request.method, request.jsonrpc, request.id
        );
        error!(
            "Request params shape: {}",
            Self::param_shape(request.params.as_ref())
        );
        error!("Full error details: {:#}", e);

        McpResponse {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id: request.id.clone(),
            result: None,
            error: Some(McpError {
                code: ERROR_INTERNAL_ERROR,
                message: "Internal server error".to_owned(),
                data: None,
            }),
        }
    }

    /// Render a safe one-line description of the request params for error logs.
    ///
    /// Tool inputs can contain user-supplied secrets (API keys pasted into a prompt,
    /// OAuth codes, passwords). Logging the full payload at error-level would leak
    /// them. This helper returns the top-level key names for object params and a
    /// type label for other shapes — enough to triage failures without exposing values.
    fn param_shape(params: Option<&serde_json::Value>) -> String {
        match params {
            None => "none".to_owned(),
            Some(serde_json::Value::Object(map)) => {
                let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
                keys.sort_unstable();
                format!("object keys=[{}]", keys.join(","))
            }
            Some(serde_json::Value::Array(arr)) => format!("array len={}", arr.len()),
            Some(serde_json::Value::String(_)) => "string".to_owned(),
            Some(serde_json::Value::Number(_)) => "number".to_owned(),
            Some(serde_json::Value::Bool(_)) => "bool".to_owned(),
            Some(serde_json::Value::Null) => "null".to_owned(),
        }
    }

    /// Build an MCP error response for the given request
    fn mcp_error(request: &McpRequest, code: i32, message: &str) -> McpResponse {
        McpResponse {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id: request.id.clone(),
            result: None,
            error: Some(McpError {
                code,
                message: message.to_owned(),
                data: None,
            }),
        }
    }

    /// Process an MCP request and generate response
    async fn process_request(&self, request: McpRequest) -> AppResult<McpResponse> {
        // Validate request format
        Self::validate_request(&request)?;

        // Route to appropriate handler based on method
        match request.method.as_str() {
            "initialize" => Ok(Self::handle_initialize(&request)),
            "ping" => Ok(Self::handle_ping(&request)),
            "tools/list" => Ok(self.handle_tools_list(&request).await),
            "tools/call" => self.handle_tools_call(&request).await,
            "authenticate" => Ok(Self::handle_authenticate(&request)),
            "resources/list" => Ok(self.handle_resources_list(&request).await),
            "resources/read" => Ok(self.handle_resources_read(&request).await),
            "prompts/list" => Ok(Self::handle_prompts_list(&request)),
            "prompts/get" => Ok(Self::handle_prompts_get(&request)),
            method if method.starts_with("resources/") || method.starts_with("prompts/") => {
                Ok(Self::handle_unknown_method(&request))
            }
            method if method.starts_with("sampling/") => self.handle_sampling(&request).await,
            method if method.starts_with("completion/") => Ok(Self::handle_completion(&request)),
            method if method.starts_with("roots/") => Ok(Self::handle_roots(&request)),
            _ => Ok(Self::handle_unknown_method(&request)),
        }
    }

    /// Validate MCP request format and required fields
    fn validate_request(request: &McpRequest) -> AppResult<()> {
        if request.jsonrpc != JSONRPC_VERSION {
            return Err(AppError::invalid_input(format!(
                "Invalid JSON-RPC version: got '{}', expected '{}'",
                request.jsonrpc, JSONRPC_VERSION
            )));
        }

        if request.method.is_empty() {
            return Err(AppError::invalid_input("Missing method"));
        }

        Ok(())
    }

    /// Supported MCP protocol versions, in preference order.
    const SUPPORTED_VERSIONS: &'static [&'static str] = &["2025-11-25", "2025-06-18", "2024-11-05"];

    /// Handle MCP initialize request with protocol version negotiation.
    ///
    /// Echoes the client's requested version when supported, otherwise returns
    /// a version-mismatch error listing the supported versions.
    #[must_use]
    pub fn handle_initialize(request: &McpRequest) -> McpResponse {
        debug!("Handling initialize request");

        let request_id = request.id.clone().unwrap_or_else(|| serde_json::json!(0));

        let Some(init_request) = request
            .params
            .as_ref()
            .and_then(|params| serde_json::from_value::<InitializeRequest>(params.clone()).ok())
        else {
            return McpResponse::error(
                Some(request_id),
                ERROR_INVALID_PARAMS,
                "Invalid initialize request parameters".to_owned(),
            );
        };

        // Negotiate the protocol version: use the client's version when supported.
        let client_version = &init_request.protocol_version;
        let negotiated_version = if Self::SUPPORTED_VERSIONS.contains(&client_version.as_str()) {
            client_version.clone()
        } else {
            let supported_versions = Self::SUPPORTED_VERSIONS.join(", ");
            return McpResponse::error(
                Some(request_id),
                ERROR_VERSION_MISMATCH,
                format!("{MSG_VERSION_MISMATCH}. Client version: {client_version}, Supported versions: {supported_versions}"),
            );
        };

        info!(
            "MCP version negotiated: {} (client: {}, server supports: {:?})",
            negotiated_version,
            client_version,
            Self::SUPPORTED_VERSIONS
        );

        // Resolve the public OAuth origin for the advertised capability endpoints.
        let base_url = get_server_config().map_or_else(
            || "http://localhost:8081".to_owned(),
            |c| c.base_url.clone(),
        );
        let init_response = InitializeResponse::new(
            negotiated_version,
            server_name_multitenant(),
            SERVER_VERSION.to_owned(),
            &base_url,
        );

        match serde_json::to_value(&init_response) {
            Ok(result) => McpResponse::success(Some(request_id), result),
            Err(e) => {
                error!("Failed to serialize initialize response: {}", e);
                McpResponse::error(
                    Some(request_id),
                    ERROR_SERIALIZATION,
                    format!("{MSG_SERIALIZATION}: {e}"),
                )
            }
        }
    }

    /// Handle MCP ping request
    #[must_use]
    pub fn handle_ping(request: &McpRequest) -> McpResponse {
        debug!("Handling ping request");

        McpResponse {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id: request.id.clone(),
            result: Some(serde_json::json!({})),
            error: None,
        }
    }

    /// Handle MCP authenticate request
    fn handle_authenticate(request: &McpRequest) -> McpResponse {
        debug!("Handling authenticate request");

        // Always return authentication parameter error for authenticate method
        McpResponse {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id: request.id.clone(),
            result: None,
            error: Some(McpError {
                code: -32602, // Invalid params
                message: "Invalid authentication parameters".to_owned(),
                data: None,
            }),
        }
    }

    /// Handle tools/list request with tiered visibility based on authentication state
    ///
    /// Tool discovery is filtered by authentication context:
    /// - **Unauthenticated**: Returns only public discovery tools (safe, read-only capabilities)
    /// - **Authenticated + tenant**: Returns tenant-filtered tools via `ToolSelectionService`
    /// - **Authenticated + admin**: Returns all tools including admin tools
    ///
    /// This ensures sensitive tools (connection management, admin operations, future social tools)
    /// are not exposed to unauthenticated MCP clients while still allowing capability discovery.
    async fn handle_tools_list(&self, request: &McpRequest) -> McpResponse {
        debug!("Handling tools/list request");

        let tools = self.resolve_tools_for_request(request).await;

        McpResponse {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id: request.id.clone(),
            result: Some(serde_json::json!({ "tools": tools })),
            error: None,
        }
    }

    /// Resolve which tools to return based on authentication state in the request
    async fn resolve_tools_for_request(&self, request: &McpRequest) -> Vec<ToolSchema> {
        // Extract auth token (same pattern as tools/call in tool_handlers.rs)
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

        let Some(token) = auth_token else {
            debug!("tools/list: no auth token, returning public discovery tools");
            return self.public_discovery_tools();
        };

        match self
            .resources
            .auth
            .auth_middleware
            .authenticate_request(Some(token))
            .await
        {
            Ok(auth_result) => {
                info!(
                    "tools/list: authenticated user {} (method: {})",
                    auth_result.user_id,
                    auth_result.auth_method.display_name()
                );
                self.resolve_tools_for_authenticated_user(
                    auth_result.user_id,
                    auth_result.active_tenant_id.map(TenantId::from),
                )
                .await
            }
            Err(e) => {
                debug!(
                    "tools/list: auth failed ({}), returning public discovery tools",
                    e
                );
                self.public_discovery_tools()
            }
        }
    }

    /// Resolve tools for an authenticated user based on their tenant context
    async fn resolve_tools_for_authenticated_user(
        &self,
        user_id: uuid::Uuid,
        active_tenant_id: Option<TenantId>,
    ) -> Vec<ToolSchema> {
        if let Ok(Some(tenant_ctx)) = extract_tenant_context_internal(
            &self.resources.common.repos,
            Some(user_id),
            active_tenant_id,
            None,
        )
        .await
        {
            if tenant_ctx.is_admin() {
                debug!(
                    "tools/list: admin user in tenant {}, returning all tools",
                    tenant_ctx.tenant_id
                );
                return self.resources.mcp.tool_registry.all_schemas();
            }

            debug!(
                "tools/list: user in tenant {}, returning tenant-filtered tools",
                tenant_ctx.tenant_id
            );
            return self.tenant_filtered_tools(tenant_ctx.tenant_id).await;
        }

        // Authenticated but no tenant context: return user-visible tools
        // (non-admin tools from registry, no tenant filtering)
        debug!("tools/list: authenticated user without tenant, returning user-visible tools");
        self.resources.mcp.tool_registry.user_visible_schemas()
    }

    /// Return public discovery tools (safe subset for unauthenticated clients)
    fn public_discovery_tools(&self) -> Vec<ToolSchema> {
        self.resources
            .mcp
            .tool_registry
            .list_schemas_by_names(PUBLIC_DISCOVERY_TOOLS)
    }

    /// Get tenant-filtered tool schemas for non-admin users
    ///
    /// Combines three sources to build the tool list:
    /// 1. Enabled tools from `ToolSelectionService` (catalog-based, tenant-aware)
    /// 2. Uncatalogued tools from the registry (feature-flag tools like coaches/mobility)
    /// 3. `PUBLIC_DISCOVERY_TOOLS` as a floor — an authenticated user must never see
    ///    fewer tools than an anonymous caller, regardless of plan restrictions. Tools
    ///    in the public set whose catalog `min_plan` exceeds the tenant's plan would
    ///    otherwise disappear from `tools/list` entirely for lower-tier tenants. They
    ///    will still be rejected at `tools/call` time via plan enforcement, but they
    ///    remain discoverable.
    ///
    /// Admin-only tools are excluded to prevent non-admin users from seeing them
    /// even if they appear in the catalog.
    async fn tenant_filtered_tools(&self, tenant_id: TenantId) -> Vec<ToolSchema> {
        match self
            .resources
            .mcp
            .tool_selection
            .get_effective_tools(tenant_id)
            .await
        {
            Ok(all_effective_tools) => {
                // Separate enabled tool names from full catalog for uncatalogued detection
                let enabled_names: Vec<String> = all_effective_tools
                    .iter()
                    .filter(|t| t.is_enabled)
                    .map(|t| t.tool_name.clone())
                    .collect();
                let all_catalogued_names: Vec<String> = all_effective_tools
                    .into_iter()
                    .map(|t| t.tool_name)
                    .collect();

                // Get catalog-based enabled tools, filtered to non-admin only
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
                        .is_none_or(|tool| !tool.capabilities().is_admin_only())
                });

                // Include feature-flag tools not tracked by tool_catalog (coaches, mobility, etc.)
                // Uses all_catalogued_names so disabled-in-catalog tools aren't re-added
                let uncatalogued = self
                    .resources
                    .mcp
                    .tool_registry
                    .uncatalogued_user_schemas(&all_catalogued_names);
                schemas.extend(uncatalogued);

                // Ensure the public-discovery floor is present even when plan
                // restrictions disable some of those tools in the catalog.
                let present: HashSet<&str> = schemas.iter().map(|s| s.name.as_str()).collect();
                let missing_public: Vec<&str> = PUBLIC_DISCOVERY_TOOLS
                    .iter()
                    .copied()
                    .filter(|name| !present.contains(name))
                    .collect();
                if !missing_public.is_empty() {
                    let floor = self
                        .resources
                        .mcp
                        .tool_registry
                        .list_schemas_by_names(&missing_public);
                    schemas.extend(floor);
                }

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

    /// Handle tools/call request
    async fn handle_tools_call(&self, request: &McpRequest) -> AppResult<McpResponse> {
        debug!("Handling tools/call request");

        request
            .params
            .as_ref()
            .ok_or_else(|| AppError::invalid_input("Missing parameters for tools/call"))?;

        // Execute tool using static method - delegate to ToolHandlers
        // Clone the entire request and reset metadata
        let mut handler_request = request.clone();
        handler_request.metadata = HashMap::new();
        let response =
            ToolHandlers::handle_tools_call_with_resources(handler_request, &self.resources).await;
        Ok(response)
    }

    /// Handle the resources/list MCP method.
    ///
    /// Lists the global coach marketplace — only publicly published coaches are
    /// exposed, with no tenant or auth threading. Each coach surfaces as a
    /// `dravr://coaches/{id}` `text/markdown` resource.
    async fn handle_resources_list(&self, request: &McpRequest) -> McpResponse {
        debug!("Handling resources/list request");

        let published = match self
            .resources
            .store_listings_repository()
            .get_published_coaches(None, None, Some(resource_catalog::list_limit()), Some(0))
            .await
        {
            Ok(coaches) => coaches,
            Err(e) => {
                error!("resources/list: failed to load published coaches: {e}");
                return Self::mcp_error(
                    request,
                    ERROR_INTERNAL_ERROR,
                    "Failed to load coach catalog",
                );
            }
        };

        McpResponse {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id: request.id.clone(),
            result: Some(resource_catalog::list_resources(&published)),
            error: None,
        }
    }

    /// Handle the resources/read MCP method.
    ///
    /// Reads a single published coach addressed by a `dravr://coaches/{id}`
    /// URI and returns its reconstructed markdown document.
    async fn handle_resources_read(&self, request: &McpRequest) -> McpResponse {
        debug!("Handling resources/read request");

        let Some(uri) = request
            .params
            .as_ref()
            .and_then(|params| params.get("uri"))
            .and_then(|uri| uri.as_str())
        else {
            return Self::mcp_error(request, ERROR_INVALID_PARAMS, "Missing 'uri' parameter");
        };

        let Some(coach_id) = resource_catalog::coach_id_from_uri(uri) else {
            return Self::mcp_error(
                request,
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
                return Self::mcp_error(request, ERROR_INVALID_PARAMS, "Resource not found");
            }
            Err(e) => {
                error!("resources/read: failed to load coach: {e}");
                return Self::mcp_error(request, ERROR_INTERNAL_ERROR, "Failed to load coach");
            }
        };

        match resource_catalog::read_resource(&published.coach) {
            Ok(result) => McpResponse {
                jsonrpc: JSONRPC_VERSION.to_owned(),
                id: request.id.clone(),
                result: Some(result),
                error: None,
            },
            Err(e) => {
                error!("resources/read: failed to render coach markdown: {e}");
                Self::mcp_error(request, ERROR_INTERNAL_ERROR, "Failed to render resource")
            }
        }
    }

    /// Handle the prompts/list MCP method.
    ///
    /// Advertises the curated, user-invokable analysis prompt templates with
    /// their typed argument schemas.
    fn handle_prompts_list(request: &McpRequest) -> McpResponse {
        debug!("Handling prompts/list request");

        McpResponse {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id: request.id.clone(),
            result: Some(prompt_templates::list_prompts()),
            error: None,
        }
    }

    /// Handle the prompts/get MCP method.
    ///
    /// Assembles the prompt messages for a curated template given the
    /// client-supplied argument values.
    fn handle_prompts_get(request: &McpRequest) -> McpResponse {
        debug!("Handling prompts/get request");

        let Some(name) = request
            .params
            .as_ref()
            .and_then(|params| params.get("name"))
            .and_then(|name| name.as_str())
        else {
            return Self::mcp_error(request, ERROR_INVALID_PARAMS, "Missing 'name' parameter");
        };

        let arguments = Self::extract_prompt_arguments(request);

        match prompt_templates::get_prompt(name, &arguments) {
            PromptGetOutcome::Found(result) => McpResponse {
                jsonrpc: JSONRPC_VERSION.to_owned(),
                id: request.id.clone(),
                result: Some(result),
                error: None,
            },
            PromptGetOutcome::UnknownPrompt(prompt) => Self::mcp_error(
                request,
                ERROR_INVALID_PARAMS,
                &format!("Unknown prompt: {prompt}"),
            ),
            PromptGetOutcome::MissingArgument { prompt, argument } => Self::mcp_error(
                request,
                ERROR_INVALID_PARAMS,
                &format!("Prompt '{prompt}' requires argument '{argument}'"),
            ),
        }
    }

    /// Extract the `arguments` object from a prompts/get request into a string map.
    ///
    /// Non-string argument values are stringified so templates always receive a
    /// usable value; missing or malformed `arguments` yields an empty map.
    fn extract_prompt_arguments(request: &McpRequest) -> HashMap<String, String> {
        request
            .params
            .as_ref()
            .and_then(|params| params.get("arguments"))
            .and_then(|arguments| arguments.as_object())
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

    /// Handle completion requests
    fn handle_completion(request: &McpRequest) -> McpResponse {
        debug!("Handling completion request: {}", request.method);

        match request.method.as_str() {
            "completion/complete" => Self::handle_completion_complete(request),
            _ => Self::handle_unknown_method(request),
        }
    }

    /// Handle the completion/complete request for auto-complete suggestions.
    #[must_use]
    pub fn handle_completion_complete(request: &McpRequest) -> McpResponse {
        let request_id = request.id.clone().unwrap_or_else(|| serde_json::json!(0));

        // Parse the completion request
        let complete_request: Result<CompleteRequest, _> = request
            .params
            .as_ref()
            .ok_or("Missing parameters")
            .and_then(|p| serde_json::from_value(p.clone()).map_err(|_| "Invalid parameters"));

        match complete_request {
            Ok(req) => {
                // Generate completions based on the reference type
                let completion = Self::generate_completions(&req);

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

    /// Generate completion suggestions based on the request
    fn generate_completions(req: &CompleteRequest) -> Completion {
        // Match on the reference type
        match req.ref_.type_.as_str() {
            "ref/prompt" => {
                // Complete prompt arguments
                if req.argument.name == "activity_type" {
                    let activity_types = ["run", "ride", "swim", "strength", "walk", "hike"];
                    let matching: Vec<String> = activity_types
                        .iter()
                        .filter(|t| t.starts_with(&req.argument.value))
                        .map(|s| (*s).to_owned())
                        .collect();

                    return Completion {
                        values: matching.clone(),
                        total: Some(matching.len()),
                        has_more: Some(false),
                    };
                }

                if req.argument.name == "provider" {
                    let providers = ["strava", "fitbit", "garmin", "whoop", "terra", "sciotte"];
                    let matching: Vec<String> = providers
                        .iter()
                        .filter(|p| p.starts_with(&req.argument.value))
                        .map(|s| (*s).to_owned())
                        .collect();

                    return Completion {
                        values: matching.clone(),
                        total: Some(matching.len()),
                        has_more: Some(false),
                    };
                }

                if req.argument.name == "goal_type" {
                    let goal_types = ["distance", "time", "frequency", "performance", "custom"];
                    let matching: Vec<String> = goal_types
                        .iter()
                        .filter(|g| g.starts_with(&req.argument.value))
                        .map(|s| (*s).to_owned())
                        .collect();

                    return Completion {
                        values: matching.clone(),
                        total: Some(matching.len()),
                        has_more: Some(false),
                    };
                }
            }
            "ref/resource" => {
                // Complete resource URIs
                let resource_uris = ["oauth://notifications"];
                let matching: Vec<String> = resource_uris
                    .iter()
                    .filter(|uri| uri.starts_with(&req.argument.value))
                    .map(|s| (*s).to_owned())
                    .collect();

                return Completion {
                    values: matching.clone(),
                    total: Some(matching.len()),
                    has_more: Some(false),
                };
            }
            _ => {}
        }

        // Default: no completions
        Completion {
            values: vec![],
            total: Some(0),
            has_more: Some(false),
        }
    }

    /// Handle roots requests
    fn handle_roots(request: &McpRequest) -> McpResponse {
        debug!("Handling roots request: {}", request.method);

        match request.method.as_str() {
            "roots/list" => Self::handle_roots_list(request),
            _ => Self::handle_unknown_method(request),
        }
    }

    /// Handle the roots/list request.
    ///
    /// Returns an empty list of roots; Dravr does not expose filesystem roots
    /// to MCP clients.
    fn handle_roots_list(request: &McpRequest) -> McpResponse {
        let request_id = request.id.clone().unwrap_or_else(|| serde_json::json!(0));
        let roots: Vec<Root> = vec![];
        let result = serde_json::json!({ "roots": roots });
        McpResponse::success(Some(request_id), result)
    }

    /// Handle sampling requests (server-initiated LLM calls)
    async fn handle_sampling(&self, request: &McpRequest) -> Result<McpResponse, AppError> {
        match request.method.as_str() {
            "sampling/createMessage" => self.handle_create_message(request).await,
            _ => Ok(Self::handle_unknown_method(request)),
        }
    }

    /// Handle the sampling/createMessage MCP method
    async fn handle_create_message(&self, request: &McpRequest) -> Result<McpResponse, AppError> {
        let Some(sampling_peer) = &self.resources.sse.sampling_peer else {
            return Ok(Self::mcp_error(
                request,
                ERROR_METHOD_NOT_FOUND,
                "Sampling not available (stdio transport only)",
            ));
        };

        let Some(params) = &request.params else {
            return Ok(Self::mcp_error(
                request,
                -32602,
                "Missing sampling parameters",
            ));
        };

        let create_message_request =
            match serde_json::from_value::<CreateMessageRequest>(params.clone()) {
                Ok(req) => req,
                Err(e) => {
                    error!("Failed to parse sampling parameters: {e}");
                    return Ok(Self::mcp_error(
                        request,
                        -32602,
                        "Invalid sampling parameters",
                    ));
                }
            };

        match sampling_peer.create_message(create_message_request).await {
            Ok(result) => match serde_json::to_value(&result) {
                Ok(result_value) => Ok(McpResponse {
                    jsonrpc: JSONRPC_VERSION.to_owned(),
                    id: request.id.clone(),
                    result: Some(result_value),
                    error: None,
                }),
                Err(e) => {
                    error!("Failed to serialize sampling result: {e}");
                    Ok(Self::mcp_error(
                        request,
                        ERROR_INTERNAL_ERROR,
                        "Failed to serialize sampling result",
                    ))
                }
            },
            Err(e) => {
                error!("Sampling request failed: {e}");
                Ok(Self::mcp_error(
                    request,
                    ERROR_INTERNAL_ERROR,
                    "Sampling request failed",
                ))
            }
        }
    }

    /// Handle unknown method
    fn handle_unknown_method(request: &McpRequest) -> McpResponse {
        warn!("Unknown MCP method: {}", request.method);

        McpResponse {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id: request.id.clone(),
            result: None,
            error: Some(McpError {
                code: ERROR_METHOD_NOT_FOUND,
                message: format!("Unknown method: {}", request.method),
                data: None,
            }),
        }
    }

    /// Handle notification (no response required)
    async fn handle_notification(&self, request: &McpRequest) {
        debug!("Handling notification: {}", request.method);

        match request.method.as_str() {
            "notifications/cancelled" => self.handle_cancelled_notification(request).await,
            "notifications/progress" => Self::handle_progress_notification(),
            "notifications/initialized" => Self::handle_initialized_notification(),
            "notifications/roots/listChanged" => Self::handle_roots_list_changed_notification(),
            _ => Self::handle_unknown_notification(&request.method),
        }
    }

    /// Handle cancelled notification
    async fn handle_cancelled_notification(&self, request: &McpRequest) {
        debug!("Request cancelled notification received");

        // Extract progress token from params
        if let Some(params) = &request.params {
            if let Some(progress_token) = params.get("progressToken").and_then(|v| v.as_str()) {
                self.resources
                    .cancel_by_progress_token(progress_token)
                    .await;
            } else {
                warn!("Cancelled notification missing progressToken parameter");
            }
        } else {
            warn!("Cancelled notification missing params");
        }
    }

    /// Handle progress notification
    fn handle_progress_notification() {
        debug!("Progress notification received");
    }

    /// Handle initialized notification (client confirms initialization complete)
    fn handle_initialized_notification() {
        debug!("Client initialization complete");
    }

    /// Handle roots/listChanged notification (client's workspace roots changed)
    fn handle_roots_list_changed_notification() {
        debug!("Client roots list changed");
    }

    /// Handle unknown notification type
    fn handle_unknown_notification(method: &str) {
        warn!(
            notification_type = %method,
            "Received unhandled notification type - may need implementation"
        );
    }

    /// Log incoming request with optional truncation
    fn log_request(request: &McpRequest) {
        let should_truncate = true; // MCP request logging is always truncated for security

        if should_truncate {
            let request_summary = format!("{}(id={:?})", request.method, request.id);
            debug!(
                mcp_method = %request.method,
                mcp_id = ?request.id,
                mcp_params_preview = ?request.params.as_ref().map(|p| {
                    let s = p.to_string();
                    if s.len() > 100 {
                        format!("{}...[truncated]", &s[..100])
                    } else {
                        s
                    }
                }),
                auth_present = request.auth_token.is_some(),
                "Received MCP request: {}",
                request_summary
            );
        } else {
            debug!(
                mcp_request = ?request,
                "Received MCP request (full)"
            );
        }
    }

    /// Log request completion with timing
    fn log_completion(request_type: &str, start_time: Instant) {
        let duration = start_time.elapsed();
        debug!(
            duration_ms = u64::try_from(duration.as_millis()).unwrap_or(0),
            "Completed MCP {} processing", request_type
        );
    }
}

/// Write MCP response to stdout for stdio transport
///
/// # Errors
/// Returns an error if JSON serialization fails or I/O operations fail
pub async fn write_response_to_stdout(
    response: &McpResponse,
    stdout: &Arc<Mutex<Stdout>>,
) -> AppResult<()> {
    let response_json = serde_json::to_string(response)
        .map_err(|e| AppError::internal(format!("JSON serialization failed: {e}")))?;
    debug!("Sending MCP response (size: {} bytes)", response_json.len());

    {
        let mut stdout_lock = stdout.lock().await;
        stdout_lock
            .write_all(response_json.as_bytes())
            .await
            .map_err(|e| AppError::internal(format!("Transport error: {e}")))?;
        stdout_lock
            .write_all(b"\n")
            .await
            .map_err(|e| AppError::internal(format!("Transport error: {e}")))?;
        stdout_lock
            .flush()
            .await
            .map_err(|e| AppError::internal(format!("Transport error: {e}")))?;
        drop(stdout_lock);
    }

    Ok(())
}
