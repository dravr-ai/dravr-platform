// ABOUTME: Core A2A protocol message handling and JSON-RPC implementation
// ABOUTME: Processes A2A protocol requests, tool execution, and task management for agent communication
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A2A Protocol Implementation
//!
// NOTE: All `.clone()` calls in this file are Safe - they are necessary for:
// - JSON-RPC message ownership for protocol serialization
// - Request/response ownership across async boundaries
//!
//! Implements the core A2A (Agent-to-Agent) protocol for Pierre,
//! providing JSON-RPC 2.0 based communication between AI agents.

use crate::a2a::{
    A2AErrorResponse, A2AInitializeRequest, A2AInitializeResponse, A2ARequest, A2AResponse,
    A2A_VERSION,
};
use crate::database_plugins::{A2ARepository, OAuthTokenRepository};
use crate::mcp::resources::ServerResources;
use crate::mcp::tenant_isolation::extract_tenant_context_internal;
use crate::tools::context::{AuthMethod, ToolExecutionContext};
use crate::types::json_schemas;
pub use pierre_core::models::a2a::{A2ATask, TaskStatus};
use pierre_core::models::OAuthAppCredentials;
use serde_json::{from_value, json, to_value, Map, Number, Value};
use std::collections::HashMap;
use std::env::var;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// A2A Protocol Server implementation
pub struct A2AServer {
    /// A2A protocol version
    pub version: String,
    /// Optional server resources for MCP integration
    pub resources: Option<Arc<ServerResources>>,
}

impl A2AServer {
    /// Create a new A2A server without resources
    #[must_use]
    pub fn new() -> Self {
        Self {
            version: A2A_VERSION.to_owned(),
            resources: None,
        }
    }

    /// Create a new A2A server with server resources
    #[must_use]
    pub fn new_with_resources(resources: Arc<ServerResources>) -> Self {
        Self {
            version: A2A_VERSION.to_owned(),
            resources: Some(resources),
        }
    }

    /// Handle incoming A2A request
    pub async fn handle_request(&self, request: A2ARequest) -> A2AResponse {
        match request.method.as_str() {
            "a2a/initialize" => {
                // Use OAuth-aware initialization if authentication is provided
                if request.auth_token.is_some() && self.resources.is_some() {
                    self.handle_initialize_with_oauth(request).await
                } else {
                    self.handle_initialize(request)
                }
            }
            "message/send" | "a2a/message/send" => Self::handle_message_send(request),
            "message/stream" | "a2a/message/stream" => self.handle_message_stream(request),
            // Authenticated endpoints: require valid JWT and pass user_id to handler
            "tasks/create" | "a2a/tasks/create" => {
                self.require_auth_then(request, |s, req, user_id| {
                    Box::pin(s.handle_task_create(req, user_id))
                })
                .await
            }
            "tasks/get" | "a2a/tasks/get" => {
                self.require_auth_then(request, |s, req, user_id| {
                    Box::pin(s.handle_task_get(req, user_id))
                })
                .await
            }
            "tasks/cancel" => Self::handle_task_cancel(request),
            "tasks/resubscribe" | "a2a/tasks/resubscribe" => self.handle_task_resubscribe(request),
            "tasks/pushNotificationConfig/set" => Self::handle_push_notification_config(request),
            "a2a/tasks/list" => {
                self.require_auth_then(request, |s, req, user_id| {
                    Box::pin(s.handle_task_list(req, user_id))
                })
                .await
            }
            "tools/list" | "a2a/tools/list" => self.handle_tools_list(request),
            "tools/call" | "a2a/tools/call" => {
                self.require_auth_then(request, |s, req, user_id| {
                    Box::pin(s.handle_tool_call(req, user_id))
                })
                .await
            }
            _ => Self::handle_unknown_method(request),
        }
    }

    /// Require authentication before dispatching to a handler
    ///
    /// Extracts and validates the JWT auth token from the request, then calls the
    /// handler with the authenticated user ID. Returns an auth error response if
    /// authentication fails.
    async fn require_auth_then<F>(&self, request: A2ARequest, handler: F) -> A2AResponse
    where
        F: FnOnce(
            &Self,
            A2ARequest,
            Uuid,
        ) -> Pin<Box<dyn Future<Output = A2AResponse> + Send + '_>>,
    {
        let Some(resources) = &self.resources else {
            return A2AResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(A2AErrorResponse {
                    code: -32000,
                    message: "A2A server not properly configured".into(),
                    data: None,
                }),
                id: request.id.clone(),
            };
        };
        match Self::authenticate_request(&request, resources) {
            Ok(user_id) => handler(self, request, user_id).await,
            Err(err_response) => *err_response,
        }
    }

    fn handle_initialize(&self, request: A2ARequest) -> A2AResponse {
        let result = json!({
            "version": self.version,
            "capabilities": [
                "message/send",
                "message/stream",
                "tasks/create",
                "tasks/get",
                "tasks/cancel",
                "tasks/pushNotificationConfig/set",
                "tools/list",
                "tools/call"
            ],
            "agent": {
                "name": "Pierre Fitness Intelligence Agent",
                "version": "1.0.0",
                "description": "AI-powered fitness data analysis and insights platform"
            }
        });

        A2AResponse {
            jsonrpc: "2.0".into(),
            result: Some(result),
            error: None,
            id: request.id,
        }
    }

    /// Handle A2A initialize request with `ServerResources` for OAuth credential storage
    async fn handle_initialize_with_oauth(&self, request: A2ARequest) -> A2AResponse {
        // Extract resources with defensive error handling
        let Some(resources) = self.resources.as_ref() else {
            return A2AResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(A2AErrorResponse {
                    code: -32603,
                    message: "Internal error: Server resources not initialized".to_owned(),
                    data: None,
                }),
                id: request.id.clone(),
            };
        };

        let user_id = match Self::authenticate_request(&request, resources) {
            Ok(user_id) => user_id,
            Err(error_response) => return *error_response,
        };

        // Handle OAuth credentials if provided
        let response = self.handle_initialize_internal(request.clone()); // Safe: Request ownership for internal call

        // If initialization successful and OAuth credentials provided, store them
        if response.error.is_none() {
            if let Some(params) = &request.params {
                if let Ok(init_request) = from_value::<A2AInitializeRequest>(params.clone())
                // Safe: JSON value ownership for deserialization
                {
                    if let Some(oauth_creds) = init_request.oauth_credentials {
                        if let Err(e) =
                            Self::store_oauth_credentials(oauth_creds, &user_id, resources).await
                        {
                            warn!(
                                "Failed to store OAuth credentials during A2A initialization: {e}"
                            );
                        } else {
                            info!("Successfully stored OAuth credentials for A2A user {user_id}");
                        }
                    }
                }
            }
        }

        response
    }

    /// Internal A2A initialize handler with proper protocol version negotiation
    fn handle_initialize_internal(&self, request: A2ARequest) -> A2AResponse {
        // Parse A2A initialize request parameters
        let init_request = request.params.as_ref().and_then(|params| {
            from_value::<A2AInitializeRequest>(params.clone()) // Safe: JSON value ownership for deserialization
                .inspect_err(|e| {
                    warn!(
                        error = ?e,
                        "Failed to parse A2A initialize request parameters - using defaults (params redacted)"
                    );
                })
                .ok()
        });

        let (protocol_version, client_name) = if let Some(req) = init_request {
            // For now, accept any protocol version - A2A is more flexible
            (req.protocol_version, req.client_info.name)
        } else {
            // Default values if parsing fails
            debug!("A2A initialize: using default values (no valid params provided)");
            (A2A_VERSION.to_owned(), "unknown".to_owned())
        };

        info!("A2A initialization from client: {client_name} with protocol version: {protocol_version}");

        // Create A2A initialize response
        let init_response = A2AInitializeResponse::new(
            protocol_version,
            "pierre-a2a-server".to_owned(),
            self.version.clone(), // Safe: String ownership for response
        );

        match to_value(&init_response) {
            Ok(result) => A2AResponse {
                jsonrpc: "2.0".into(),
                result: Some(result),
                error: None,
                id: request.id,
            },
            Err(e) => A2AResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(A2AErrorResponse {
                    code: -32603,
                    message: format!("Failed to serialize A2A initialize response: {e}"),
                    data: None,
                }),
                id: request.id,
            },
        }
    }

    /// Authenticate the A2A request and extract user information
    fn authenticate_request(
        request: &A2ARequest,
        resources: &Arc<ServerResources>,
    ) -> Result<Uuid, Box<A2AResponse>> {
        let request_id = request
            .id
            .clone() // Safe: JSON value ownership for request ID
            .unwrap_or_else(|| Value::Number(Number::from(0)));

        // Extract auth token from request
        let auth_token = request.auth_token.as_deref().ok_or_else(|| {
            Box::new(A2AResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(A2AErrorResponse {
                    code: -32001,
                    message: "Authentication token required for OAuth credential storage"
                        .to_owned(),
                    data: None,
                }),
                id: Some(request_id.clone()), // Safe: JSON value ownership for error response
            })
        })?;

        // Validate token and extract user_id
        match resources
            .auth_manager
            .validate_token(auth_token, &resources.jwks_manager)
        {
            Ok(claims) => Uuid::parse_str(&claims.sub).map_or_else(
                |_| {
                    Err(Box::new(A2AResponse {
                        jsonrpc: "2.0".into(),
                        result: None,
                        error: Some(A2AErrorResponse {
                            code: -32001,
                            message: "Invalid user ID in authentication token".to_owned(),
                            data: None,
                        }),
                        id: Some(request_id.clone()), // Safe: JSON value ownership for error response
                    }))
                },
                Ok,
            ),
            Err(_) => Err(Box::new(A2AResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(A2AErrorResponse {
                    code: -32001,
                    message: "Invalid authentication token".to_owned(),
                    data: None,
                }),
                id: Some(request_id),
            })),
        }
    }

    /// Get client IDs owned by a user
    async fn get_owned_client_ids(
        user_id: &Uuid,
        resources: &Arc<ServerResources>,
    ) -> Result<Vec<String>, String> {
        resources
            .database
            .list_clients(user_id)
            .await
            .map(|clients| clients.into_iter().map(|c| c.id).collect())
            .map_err(|e| format!("Failed to list A2A clients: {e}"))
    }

    /// Verify a `client_id` belongs to the authenticated user
    async fn verify_client_ownership(
        client_id: &str,
        user_id: &Uuid,
        resources: &Arc<ServerResources>,
        request_id: Option<&Value>,
    ) -> Result<(), A2AResponse> {
        let owned_ids = Self::get_owned_client_ids(user_id, resources)
            .await
            .map_err(|e| A2AResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(A2AErrorResponse {
                    code: -32000,
                    message: format!("Failed to resolve client ownership: {e}"),
                    data: None,
                }),
                id: request_id.cloned(),
            })?;

        if !owned_ids.iter().any(|id| id == client_id) {
            return Err(Self::permission_denied_error(request_id.cloned()));
        }
        Ok(())
    }

    /// Create a standard permission denied error response
    fn permission_denied_error(request_id: Option<Value>) -> A2AResponse {
        A2AResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(A2AErrorResponse {
                code: -32001,
                message: "Permission denied: client does not belong to authenticated user".into(),
                data: None,
            }),
            id: request_id,
        }
    }

    /// Create a standard server-not-configured error response
    fn server_not_configured_error(request_id: Option<Value>) -> A2AResponse {
        A2AResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(A2AErrorResponse {
                code: -32000,
                message: "A2A server not properly configured".into(),
                data: None,
            }),
            id: request_id,
        }
    }

    /// Store OAuth credentials provided during A2A initialization
    async fn store_oauth_credentials(
        oauth_creds: HashMap<String, OAuthAppCredentials>,
        user_id: &Uuid,
        resources: &Arc<ServerResources>,
    ) -> Result<(), String> {
        for (provider, creds) in oauth_creds {
            info!("Storing OAuth credentials for provider {provider} for A2A user {user_id}");

            // Store encrypted OAuth app credentials in database
            // Use default redirect URI for A2A clients
            let redirect_uri = format!("urn:ietf:wg:oauth:2.0:oob:{provider}:a2a");
            resources
                .database
                .store_user_oauth_app(
                    *user_id,
                    &provider,
                    &creds.client_id,
                    &creds.client_secret,
                    &redirect_uri,
                )
                .await
                .map_err(|e| format!("Failed to store {provider} OAuth credentials: {e}"))?;
        }

        Ok(())
    }

    /// Resolve the base URL from server config, falling back to `BASE_URL` env var,
    /// then to the default `http://localhost:8081`.
    fn resolve_base_url(&self) -> String {
        self.resources.as_ref().map_or_else(
            || var("BASE_URL").unwrap_or_else(|_| "http://localhost:8081".to_owned()),
            |r| r.config.base_url.clone(),
        )
    }

    fn handle_message_send(request: A2ARequest) -> A2AResponse {
        // Message sending would forward requests to appropriate handlers
        A2AResponse {
            jsonrpc: "2.0".into(),
            result: Some(json!({"status": "received"})),
            error: None,
            id: request.id,
        }
    }

    fn handle_message_stream(&self, request: A2ARequest) -> A2AResponse {
        let base_url = self.resolve_base_url();

        // Extract stream_id or task_id from params (support both for flexibility)
        let params = request.params.as_ref().unwrap_or(&Value::Null);
        let stream_id = params
            .get("stream_id")
            .or_else(|| params.get("task_id"))
            .and_then(|v| v.as_str());

        if let Some(id) = stream_id {
            // Return SSE streaming endpoint for specific stream/task
            A2AResponse {
                jsonrpc: "2.0".into(),
                result: Some(json!({
                    "stream_url": format!("{}/a2a/tasks/{}/stream", base_url, id),
                    "stream_type": "text/event-stream",
                    "protocol": "SSE",
                    "keep_alive_interval_seconds": 15,
                    "status": "streaming_available"
                })),
                error: None,
                id: request.id,
            }
        } else {
            // Return generic streaming info if no specific ID provided
            A2AResponse {
                jsonrpc: "2.0".into(),
                result: Some(json!({
                    "streaming_supported": true,
                    "stream_type": "text/event-stream",
                    "protocol": "SSE",
                    "status": "available"
                })),
                error: None,
                id: request.id,
            }
        }
    }

    async fn handle_task_create(&self, request: A2ARequest, user_id: Uuid) -> A2AResponse {
        // Parse request parameters using typed struct
        let params_value = request.params.as_ref().unwrap_or(&Value::Null);

        let task_params =
            match from_value::<json_schemas::A2ATaskCreateParams>(params_value.clone()) {
                Ok(params) => params,
                Err(e) => {
                    error!("Failed to parse A2A task create parameters: {}", e);
                    return A2AResponse {
                        jsonrpc: "2.0".into(),
                        result: None,
                        error: Some(A2AErrorResponse {
                            code: -32602,
                            message: format!("Invalid parameters: {e}"),
                            data: None,
                        }),
                        id: request.id,
                    };
                }
            };

        let client_id = task_params.client_id;
        let task_type = task_params.task_type;

        // Verify the caller owns the client_id they're creating a task for
        let Some(resources) = &self.resources else {
            return Self::server_not_configured_error(request.id);
        };
        if let Err(err) =
            Self::verify_client_ownership(&client_id, &user_id, resources, request.id.as_ref())
                .await
        {
            return err;
        }

        // Persist task to database and get generated task_id
        let database = &resources.database;
        let task_id = match database
            .create_task(
                &client_id,
                None, // session_id - optional
                &task_type,
                params_value,
            )
            .await
        {
            Ok(id) => {
                info!("Created A2A task {} for client {}", id, client_id);
                id
            }
            Err(e) => {
                return A2AResponse {
                    jsonrpc: "2.0".into(),
                    result: None,
                    error: Some(A2AErrorResponse {
                        code: -32000,
                        message: format!("Failed to persist task: {e}"),
                        data: None,
                    }),
                    id: request.id,
                };
            }
        };

        let task = A2ATask {
            id: task_id,
            status: TaskStatus::Pending,
            created_at: chrono::Utc::now(),
            completed_at: None,
            result: None,
            error: None,
            client_id,
            task_type,
            input_data: params_value.clone(), // Safe: JSON value ownership for task input
            output_data: None,
            error_message: None,
            updated_at: chrono::Utc::now(),
        };

        match to_value(task) {
            Ok(task_value) => A2AResponse {
                jsonrpc: "2.0".into(),
                result: Some(task_value),
                error: None,
                id: request.id,
            },
            Err(e) => A2AResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(A2AErrorResponse {
                    code: -32603,
                    message: "Internal error: Failed to serialize task".to_owned(),
                    data: Some(json!({
                        "error": e.to_string(),
                        "context": "Task serialization failed"
                    })),
                }),
                id: request.id,
            },
        }
    }

    async fn handle_task_get(&self, request: A2ARequest, user_id: Uuid) -> A2AResponse {
        let Some(resources) = &self.resources else {
            return Self::server_not_configured_error(request.id);
        };

        // Parse task_id from params
        let params_value = request.params.as_ref().unwrap_or(&Value::Null);
        let task_params = match from_value::<json_schemas::A2ATaskGetParams>(params_value.clone()) {
            Ok(params) => params,
            Err(e) => {
                error!("Failed to parse A2A task get parameters: {}", e);
                return A2AResponse {
                    jsonrpc: "2.0".into(),
                    result: None,
                    error: Some(A2AErrorResponse {
                        code: -32602,
                        message: format!("Invalid parameters: {e}"),
                        data: None,
                    }),
                    id: request.id,
                };
            }
        };

        let task_id = &task_params.task_id;
        let database = &resources.database;

        // Get task from database
        match database.get_task(task_id).await {
            Ok(Some(task)) => {
                // Verify the task belongs to a client owned by the authenticated user
                if let Err(err) = Self::verify_client_ownership(
                    &task.client_id,
                    &user_id,
                    resources,
                    request.id.as_ref(),
                )
                .await
                {
                    return err;
                }
                A2AResponse {
                    jsonrpc: "2.0".into(),
                    result: Some(to_value(task).unwrap_or_default()),
                    error: None,
                    id: request.id,
                }
            }
            Ok(None) => A2AResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(A2AErrorResponse {
                    code: -32601,
                    message: "Task not found".into(),
                    data: None,
                }),
                id: request.id,
            },
            Err(e) => A2AResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(A2AErrorResponse {
                    code: -32000,
                    message: format!("Database error: {e}"),
                    data: None,
                }),
                id: request.id,
            },
        }
    }

    async fn handle_task_list(&self, request: A2ARequest, user_id: Uuid) -> A2AResponse {
        let Some(resources) = &self.resources else {
            return Self::server_not_configured_error(request.id);
        };

        // Get the authenticated user's owned client IDs to scope the query
        let owned_client_ids = match Self::get_owned_client_ids(&user_id, resources).await {
            Ok(ids) => ids,
            Err(e) => {
                return A2AResponse {
                    jsonrpc: "2.0".into(),
                    result: None,
                    error: Some(A2AErrorResponse {
                        code: -32000,
                        message: format!("Failed to resolve client ownership: {e}"),
                        data: None,
                    }),
                    id: request.id,
                };
            }
        };

        let database = &resources.database;
        let params_value = request.params.as_ref().unwrap_or(&Value::Null);
        let list_params = from_value::<json_schemas::A2ATaskListParams>(params_value.clone())
            .unwrap_or(json_schemas::A2ATaskListParams {
                client_id: None,
                status: None,
                limit: 20,
                offset: None,
            });

        // If caller specifies a client_id, verify they own it; otherwise use their first client
        let scoped_client_id = if let Some(ref requested) = list_params.client_id {
            if !owned_client_ids.contains(requested) {
                return Self::permission_denied_error(request.id);
            }
            Some(requested.as_str())
        } else {
            // Scope to first owned client; if user has no clients, return empty list
            owned_client_ids.first().map(String::as_str)
        };

        let limit = Some(list_params.limit);
        let offset = list_params.offset;

        let status_filter = list_params
            .status
            .as_deref()
            .and_then(|status_str| match status_str {
                "pending" => Some(TaskStatus::Pending),
                "running" => Some(TaskStatus::Running),
                "completed" => Some(TaskStatus::Completed),
                "failed" => Some(TaskStatus::Failed),
                "cancelled" => Some(TaskStatus::Cancelled),
                _ => None,
            });

        match database
            .list_tasks(scoped_client_id, status_filter.as_ref(), limit, offset)
            .await
        {
            Ok(tasks) => {
                let tasks_json = to_value(&tasks).unwrap_or_default();
                A2AResponse {
                    jsonrpc: "2.0".into(),
                    result: Some(json!({
                        "tasks": tasks_json,
                        "total": tasks.len(),
                        "limit": limit,
                        "offset": offset.unwrap_or(0)
                    })),
                    error: None,
                    id: request.id,
                }
            }
            Err(e) => A2AResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(A2AErrorResponse {
                    code: -32000,
                    message: format!("Database error: {e}"),
                    data: None,
                }),
                id: request.id,
            },
        }
    }

    fn handle_tools_list(&self, request: A2ARequest) -> A2AResponse {
        // Get tools from registry if resources are available
        let tools = self.resources.as_ref().map_or_else(
            || {
                // Fallback to minimal static list when resources unavailable
                json!([
                    {
                        "name": "get_activities",
                        "description": "Retrieve user fitness activities",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "limit": {"type": "number", "description": "Number of activities to retrieve"},
                                "before": {"type": "string", "description": "ISO date to get activities before"}
                            }
                        }
                    }
                ])
            },
            |resources| {
                // Use registry's user-visible schemas (excludes admin-only tools)
                let schemas = resources.tool_registry.user_visible_schemas();
                // Convert ToolSchema structs to JSON values
                let schema_values: Vec<Value> = schemas
                    .into_iter()
                    .filter_map(|schema| to_value(schema).ok())
                    .collect();
                Value::Array(schema_values)
            },
        );

        A2AResponse {
            jsonrpc: "2.0".into(),
            result: Some(json!({ "tools": tools })),
            error: None,
            id: request.id,
        }
    }

    async fn handle_tool_call(&self, request: A2ARequest, user_id: Uuid) -> A2AResponse {
        // Extract tool call parameters
        let params = request.params.unwrap_or_default();

        let tool_name = params
            .get("tool_name")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| {
                warn!("Missing tool_name in A2A tool execution request, using 'unknown'");
                "unknown"
            });

        let tool_params = params
            .get("parameters")
            .and_then(|v| v.as_object())
            .cloned()
            .map_or_else(|| Value::Object(Map::default()), Value::Object);

        let Some(resources) = &self.resources else {
            return Self::server_not_configured_error(request.id);
        };
        let resources = resources.clone(); // Safe: Arc clone for server resources

        // Resolve tenant context — required for all A2A tool execution
        let tenant_context =
            match extract_tenant_context_internal(&resources.database, Some(user_id), None, None)
                .await
            {
                Ok(Some(ctx)) => ctx,
                Ok(None) => {
                    return A2AResponse {
                        jsonrpc: "2.0".into(),
                        result: None,
                        error: Some(A2AErrorResponse {
                            code: -32001,
                            message: "User does not belong to any tenant".into(),
                            data: None,
                        }),
                        id: request.id,
                    };
                }
                Err(e) => {
                    return A2AResponse {
                        jsonrpc: "2.0".into(),
                        result: None,
                        error: Some(A2AErrorResponse {
                            code: -32603,
                            message: format!("Failed to resolve tenant context: {e}"),
                            data: None,
                        }),
                        id: request.id,
                    };
                }
            };

        // Build tool execution context from authenticated user identity with tenant
        let tool_ctx = ToolExecutionContext::new(
            user_id,
            Some(tenant_context.tenant_id),
            resources.clone(),
            AuthMethod::ApiKey,
        );

        // Try the registry first, fall back to error for unregistered tools
        if resources.tool_registry.contains(tool_name) {
            match resources
                .tool_registry
                .execute(tool_name, tool_params, &tool_ctx)
                .await
            {
                Ok(result) => A2AResponse {
                    jsonrpc: "2.0".into(),
                    result: Some(json!({
                        "content": [{
                            "type": "text",
                            "text": result.content.to_string()
                        }],
                        "isError": result.is_error
                    })),
                    error: None,
                    id: request.id,
                },
                Err(e) => A2AResponse {
                    jsonrpc: "2.0".into(),
                    result: None,
                    error: Some(A2AErrorResponse {
                        code: -32000,
                        message: format!("Tool execution failed: {e}"),
                        data: None,
                    }),
                    id: request.id,
                },
            }
        } else {
            A2AResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(A2AErrorResponse {
                    code: -32601,
                    message: format!("Unknown tool: {tool_name}"),
                    data: None,
                }),
                id: request.id,
            }
        }
    }

    fn handle_task_resubscribe(&self, request: A2ARequest) -> A2AResponse {
        let base_url = self.resolve_base_url();

        let params = request.params.as_ref().unwrap_or(&Value::Null);
        let task_id = params.get("task_id").and_then(|v| v.as_str());

        if let Some(task_id) = task_id {
            // Return resubscription information with new stream endpoint
            A2AResponse {
                jsonrpc: "2.0".into(),
                result: Some(json!({
                    "task_id": task_id,
                    "stream_url": format!("{}/a2a/tasks/{}/stream", base_url, task_id),
                    "stream_type": "text/event-stream",
                    "protocol": "SSE",
                    "reconnected": true,
                    "message": "Task stream available for resubscription"
                })),
                error: None,
                id: request.id,
            }
        } else {
            A2AResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(A2AErrorResponse {
                    code: -32602,
                    message: "Missing required parameter: task_id".into(),
                    data: None,
                }),
                id: request.id,
            }
        }
    }

    fn handle_task_cancel(request: A2ARequest) -> A2AResponse {
        let params = request.params.unwrap_or_default();
        let task_id = params
            .get("task_id")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| {
                warn!("Missing task_id in A2A task cancel request, using 'unknown'");
                "unknown"
            });

        // In a full implementation, this would cancel an active task
        // Simulate task cancellation until full task management is implemented
        A2AResponse {
            jsonrpc: "2.0".into(),
            result: Some(json!({
                "task_id": task_id,
                "status": "cancelled",
                "cancelled_at": chrono::Utc::now().to_rfc3339()
            })),
            error: None,
            id: request.id,
        }
    }

    fn handle_push_notification_config(request: A2ARequest) -> A2AResponse {
        let params = request.params.unwrap_or_default();

        // Extract notification configuration from params
        let config = params.get("config").cloned().unwrap_or_default();

        // In a full implementation, this would store push notification settings
        A2AResponse {
            jsonrpc: "2.0".into(),
            result: Some(json!({
                "status": "configured",
                "config": config,
                "updated_at": chrono::Utc::now().to_rfc3339()
            })),
            error: None,
            id: request.id,
        }
    }

    fn handle_unknown_method(request: A2ARequest) -> A2AResponse {
        let error = A2AErrorResponse {
            code: -32601,
            message: format!("Method not found: {}", request.method),
            data: None,
        };

        A2AResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(error),
            id: request.id,
        }
    }
}

impl Default for A2AServer {
    fn default() -> Self {
        Self::new()
    }
}
