// ABOUTME: A2A 1.0 JSON-RPC method dispatch and task lifecycle execution
// ABOUTME: PascalCase spec methods (SendMessage, GetTask, ListTasks, CancelTask, push-config CRUD)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A2A Protocol Implementation
//!
// NOTE: All `.clone()` calls in this file are Safe - they are necessary for:
// - JSON-RPC message ownership for protocol serialization
// - Request/response ownership across async boundaries
//!
//! Implements the A2A 1.0 protocol for Pierre: JSON-RPC 2.0 method dispatch
//! with the spec's `PascalCase` method names, task lifecycle persistence,
//! streaming event publication, and push notification delivery. The two
//! streaming methods (`SendStreamingMessage`, `SubscribeToTask`) are
//! initiated by the HTTP layer through [`A2AServer::start_streaming_message`]
//! and [`A2AServer::start_task_subscription`], which return the initial task
//! snapshot plus a live event receiver.

use crate::events::TASK_EVENTS;
use crate::protocol_types::{
    error_info, A2ASpecError, Artifact, GetTaskRequest, ListTasksRequest, ListTasksResponse,
    Message, Part, PartContent, PushNotificationAuthenticationInfo, PushNotificationConfigInput,
    SendMessageRequest, StreamResponse, Task, TaskArtifactUpdateEvent, TaskPushNotificationConfig,
    TaskState, TaskStatusUpdateEvent, WireTaskStatus,
};
use crate::{push, A2AErrorResponse, A2ARequest, A2AResponse};
use chrono::SecondsFormat;
use pierre_core::models::a2a::A2APushNotificationConfig;
pub use pierre_core::models::a2a::{A2ATask, TaskStatus};
use pierre_mcp_transport::tenant_isolation::extract_tenant_context_internal;
use pierre_runtime_context::A2ACtx;
use pierre_tool_runtime::protocol::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::runtime::ToolRuntime;
// Trait methods dispatched through repos.a2a Arc<dyn Trait>;
use serde_json::{from_value, json, to_value, Map, Number, Value};
use std::cmp::Reverse;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{error, info};
use uuid::Uuid;

/// Default `ListTasks` page size (spec §9: default 50, range 1..=100).
const LIST_TASKS_DEFAULT_PAGE_SIZE: u32 = 50;
/// Maximum `ListTasks` page size.
const LIST_TASKS_MAX_PAGE_SIZE: u32 = 100;

/// Bundle of runtime handles required for full A2A protocol operation.
///
/// [`A2ACtx`] supplies the auth/repos/base-url slice; `tool_runtime` is held
/// separately to avoid forcing `pierre-runtime-context` to depend on
/// `pierre-tool-runtime`. The bundle keeps `A2AServer`'s field count
/// manageable and groups the two handles that must always be wired together
/// when the server runs against real infrastructure.
pub struct A2AResources {
    /// Narrow runtime-context slice: auth manager, JWKS, repos, base URL.
    pub ctx: Arc<dyn A2ACtx>,
    /// Tool runtime façade — dispatches `SendMessage` tool intents into the
    /// shared `ToolRegistry`.
    pub tool_runtime: Arc<dyn ToolRuntime>,
}

/// A2A Protocol Server implementation
pub struct A2AServer {
    /// Optional server resources for tool execution and persistence
    pub resources: Option<A2AResources>,
}

/// Resolved state shared by the blocking, non-blocking, and streaming
/// `SendMessage` paths after the task and history have been prepared.
struct PreparedSend {
    task_id: String,
    context_id: String,
    user_message: Message,
    history: Vec<Message>,
}

/// Result of executing a message against its task.
enum SendOutcome {
    /// A named tool ran successfully and produced this result
    Tool(String, Value),
    /// No tool intent — the agent echoes the message text
    Echo,
    /// Tool execution failed with this human-readable reason
    Failure(String),
}

/// Validated `ListTasks` query: the parsed request plus derived pagination
/// state (clamped page size, `pageToken` offset, parsed timestamp filter).
struct ListTasksQuery {
    params: ListTasksRequest,
    page_size: u32,
    offset: u32,
    updated_after: Option<chrono::DateTime<chrono::Utc>>,
}

impl A2AServer {
    /// Create a new A2A server without resources
    #[must_use]
    pub const fn new() -> Self {
        Self { resources: None }
    }

    /// Create a new A2A server with server resources
    #[must_use]
    pub fn new_with_resources(ctx: Arc<dyn A2ACtx>, tool_runtime: Arc<dyn ToolRuntime>) -> Self {
        Self {
            resources: Some(A2AResources { ctx, tool_runtime }),
        }
    }

    /// Handle an incoming non-streaming A2A request.
    ///
    /// Method names are the A2A 1.0 `PascalCase` strings (identical to the
    /// `gRPC` method names). The streaming methods are dispatched by the HTTP
    /// layer (they need an SSE response); reaching them here is an error.
    pub async fn handle_request(&self, request: A2ARequest) -> A2AResponse {
        match request.method.as_str() {
            "SendMessage" => {
                self.require_auth_then(request, |s, req, user_id| {
                    Box::pin(s.handle_send_message(req, user_id))
                })
                .await
            }
            "GetTask" => {
                self.require_auth_then(request, |s, req, user_id| {
                    Box::pin(s.handle_get_task(req, user_id))
                })
                .await
            }
            "ListTasks" => {
                self.require_auth_then(request, |s, req, user_id| {
                    Box::pin(s.handle_list_tasks(req, user_id))
                })
                .await
            }
            "CancelTask" => {
                self.require_auth_then(request, |s, req, user_id| {
                    Box::pin(s.handle_cancel_task(req, user_id))
                })
                .await
            }
            "CreateTaskPushNotificationConfig" => {
                self.require_auth_then(request, |s, req, user_id| {
                    Box::pin(s.handle_create_push_config(req, user_id))
                })
                .await
            }
            "GetTaskPushNotificationConfig" => {
                self.require_auth_then(request, |s, req, user_id| {
                    Box::pin(s.handle_get_push_config(req, user_id))
                })
                .await
            }
            "ListTaskPushNotificationConfigs" => {
                self.require_auth_then(request, |s, req, user_id| {
                    Box::pin(s.handle_list_push_configs(req, user_id))
                })
                .await
            }
            "DeleteTaskPushNotificationConfig" => {
                self.require_auth_then(request, |s, req, user_id| {
                    Box::pin(s.handle_delete_push_config(req, user_id))
                })
                .await
            }
            // No differentiated authenticated card is configured on this server.
            "GetExtendedAgentCard" => {
                Self::spec_error(A2ASpecError::ExtendedAgentCardNotConfigured, request.id)
            }
            // SSE-only methods: the HTTP layer intercepts these before
            // non-streaming dispatch; a non-SSE caller lands here.
            "SendStreamingMessage" | "SubscribeToTask" => Self::a2a_error(
                -32600,
                "Streaming methods require the SSE transport",
                request.id,
            ),
            _ => Self::a2a_error(
                -32601,
                format!("Method not found: {}", request.method),
                request.id,
            ),
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
            return Self::server_not_configured_error(request.id.clone());
        };
        match Self::authenticate_request(&request, resources) {
            Ok(user_id) => handler(self, request, user_id).await,
            Err(err_response) => *err_response,
        }
    }

    /// Authenticate the A2A request and extract user information.
    ///
    /// Authentication is transport-level per the spec (the card's
    /// `securitySchemes`); failures carry the `AUTHENTICATION_REQUIRED`
    /// reason so the HTTP layer can surface a 401 status.
    fn authenticate_request(
        request: &A2ARequest,
        resources: &A2AResources,
    ) -> Result<Uuid, Box<A2AResponse>> {
        let request_id = request
            .id
            .clone() // Safe: JSON value ownership for request ID
            .unwrap_or_else(|| Value::Number(Number::from(0)));

        // Extract auth token from request
        let auth_token = request.auth_token.as_deref().ok_or_else(|| {
            Box::new(Self::auth_error(
                "Authentication token required",
                Some(request_id.clone()),
            ))
        })?;

        // Validate token and extract user_id
        match resources
            .ctx
            .auth_manager()
            .validate_token(auth_token, resources.ctx.jwks_manager())
        {
            Ok(claims) => Uuid::parse_str(&claims.sub).map_or_else(
                |_| {
                    Err(Box::new(Self::auth_error(
                        "Invalid user ID in authentication token",
                        Some(request_id.clone()),
                    )))
                },
                Ok,
            ),
            Err(_) => Err(Box::new(Self::auth_error(
                "Invalid authentication token",
                Some(request_id),
            ))),
        }
    }

    /// Get client IDs owned by a user
    async fn get_owned_client_ids(
        user_id: &Uuid,
        resources: &A2AResources,
    ) -> Result<Vec<String>, String> {
        resources
            .ctx
            .repos()
            .a2a
            .list_clients(user_id)
            .await
            .map(|clients| clients.into_iter().map(|c| c.id).collect())
            .map_err(|e| format!("Failed to list A2A clients: {e}"))
    }

    /// Verify a `client_id` belongs to the authenticated user
    async fn verify_client_ownership(
        client_id: &str,
        user_id: &Uuid,
        resources: &A2AResources,
        request_id: Option<&Value>,
    ) -> Result<(), A2AResponse> {
        let owned_ids = Self::get_owned_client_ids(user_id, resources)
            .await
            .map_err(|e| {
                error!("Failed to resolve client ownership: {e}");
                Self::a2a_error(
                    -32000,
                    "Failed to resolve client ownership",
                    request_id.cloned(),
                )
            })?;

        if !owned_ids.iter().any(|id| id == client_id) {
            // Do not reveal whether the task exists for another principal.
            return Err(Self::spec_error(
                A2ASpecError::TaskNotFound,
                request_id.cloned(),
            ));
        }
        Ok(())
    }

    /// Load a task and verify the authenticated user may access it.
    async fn load_owned_task(
        resources: &A2AResources,
        task_id: &str,
        user_id: &Uuid,
        request_id: Option<&Value>,
    ) -> Result<A2ATask, Box<A2AResponse>> {
        let task = match resources.ctx.repos().a2a.get_task(task_id).await {
            Ok(Some(task)) => task,
            Ok(None) => {
                return Err(Box::new(Self::spec_error(
                    A2ASpecError::TaskNotFound,
                    request_id.cloned(),
                )))
            }
            Err(e) => {
                error!("A2A task lookup database error: {e}");
                return Err(Box::new(Self::a2a_error(
                    -32000,
                    "Database error",
                    request_id.cloned(),
                )));
            }
        };

        Self::verify_client_ownership(&task.client_id, user_id, resources, request_id)
            .await
            .map_err(Box::new)?;

        Ok(task)
    }

    /// Create a standard A2A JSON-RPC error response
    fn a2a_error(code: i32, message: impl Into<String>, request_id: Option<Value>) -> A2AResponse {
        A2AResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(A2AErrorResponse {
                code,
                message: message.into(),
                data: None,
            }),
            id: request_id,
        }
    }

    /// Create an A2A-spec error response (`-32001..-32009`) with the
    /// mandatory `google.rpc.ErrorInfo` detail.
    fn spec_error(err: A2ASpecError, request_id: Option<Value>) -> A2AResponse {
        A2AResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(err.into()),
            id: request_id,
        }
    }

    /// Create an invalid-params (`-32602`) error response.
    fn invalid_params(message: impl Into<String>, request_id: Option<Value>) -> A2AResponse {
        Self::a2a_error(-32602, message, request_id)
    }

    /// Create an authentication-failure response. Carries the
    /// `AUTHENTICATION_REQUIRED` reason so HTTP bindings can map it to 401.
    fn auth_error(message: impl Into<String>, request_id: Option<Value>) -> A2AResponse {
        A2AResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(A2AErrorResponse {
                code: -32000,
                message: message.into(),
                data: Some(error_info("AUTHENTICATION_REQUIRED")),
            }),
            id: request_id,
        }
    }

    /// Create a standard server-not-configured error response
    fn server_not_configured_error(request_id: Option<Value>) -> A2AResponse {
        Self::a2a_error(-32000, "A2A server not properly configured", request_id)
    }

    /// Successful JSON-RPC response wrapper.
    fn ok(result: Value, request_id: Option<Value>) -> A2AResponse {
        A2AResponse {
            jsonrpc: "2.0".into(),
            result: Some(result),
            error: None,
            id: request_id,
        }
    }

    /// Current instant in the spec's ISO-8601 UTC millisecond format.
    fn now_timestamp() -> String {
        chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
    }

    /// Assemble the wire `Task` from a database record, applying
    /// `historyLength` truncation and artifact inclusion.
    fn wire_task(record: &A2ATask, history_length: Option<u32>, include_artifacts: bool) -> Task {
        let status_message = record
            .status_message
            .as_ref()
            .and_then(|value| from_value::<Message>(value.clone()).ok()); // Safe: JSON value ownership for deserialization

        let mut history = record
            .history
            .as_ref()
            .and_then(|value| from_value::<Vec<Message>>(value.clone()).ok()) // Safe: JSON value ownership for deserialization
            .unwrap_or_default();
        if let Some(limit) = history_length {
            let limit = usize::try_from(limit).unwrap_or(usize::MAX);
            if history.len() > limit {
                history.drain(..history.len() - limit);
            }
        }

        let artifacts = if include_artifacts {
            record
                .artifacts
                .as_ref()
                .and_then(|value| from_value::<Vec<Artifact>>(value.clone()).ok()) // Safe: JSON value ownership for deserialization
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        Task {
            id: record.id.clone(), // Safe: String ownership for wire object
            status: WireTaskStatus {
                state: record.status,
                message: status_message,
                timestamp: Some(
                    record
                        .updated_at
                        .to_rfc3339_opts(SecondsFormat::Millis, true),
                ),
            },
            context_id: record.context_id.clone(), // Safe: String ownership for wire object
            artifacts,
            history,
            metadata: None,
        }
    }

    /// Serialize a wire task into a success response.
    fn task_response(task: &Task, request_id: Option<Value>) -> A2AResponse {
        match to_value(task) {
            Ok(value) => Self::ok(value, request_id),
            Err(e) => {
                error!("Failed to serialize A2A task: {e}");
                Self::a2a_error(-32603, "Failed to serialize task", request_id)
            }
        }
    }

    /// Serialize a wire task into a `SendMessage` success response — the
    /// result is the `SendMessageResponse` oneof, so the task is wrapped as
    /// `{"task": ...}` (the message alternative is `{"message": ...}`).
    fn send_message_task_response(task: &Task, request_id: Option<Value>) -> A2AResponse {
        match to_value(task) {
            Ok(value) => Self::ok(json!({ "task": value }), request_id),
            Err(e) => {
                error!("Failed to serialize A2A task: {e}");
                Self::a2a_error(-32603, "Failed to serialize task", request_id)
            }
        }
    }

    // ============================== SendMessage ==============================

    /// Handle A2A 1.0 `SendMessage`.
    ///
    /// Two intents are supported, both backed by real work:
    ///
    /// - **Tool invocation.** When the message carries a `data` part with
    ///   `{"tool_name": ..., "parameters": {...}}`, the named tool executes
    ///   against the authenticated user's tenant through the shared registry
    ///   path; the output lands on the task as an artifact.
    /// - **Plain message.** A message that names no tool and references no
    ///   task is answered directly with an agent `Message` (the spec's
    ///   message-only exchange) echoing the received text.
    ///
    /// With `configuration.returnImmediately == false` (the default) the
    /// call blocks until the task reaches a terminal state and returns the
    /// final task snapshot; with `true` it returns the submitted snapshot
    /// and execution continues in the background.
    async fn handle_send_message(&self, request: A2ARequest, user_id: Uuid) -> A2AResponse {
        let Some(resources) = &self.resources else {
            return Self::server_not_configured_error(request.id);
        };

        let params: SendMessageRequest = match request
            .params
            .clone() // Safe: JSON value ownership for deserialization
            .map(from_value)
            .transpose()
        {
            Ok(Some(params)) => params,
            Ok(None) => return Self::invalid_params("Missing params: message", request.id),
            Err(e) => return Self::invalid_params(format!("Invalid params: {e}"), request.id),
        };

        if params.message.parts.is_empty() {
            return Self::invalid_params("message.parts must not be empty", request.id);
        }

        let configuration = params.configuration.clone().unwrap_or_default(); // Safe: config ownership for send flow

        // Message-only exchange: no tool intent and no task reference.
        if Self::extract_tool_intent(&params.message).is_none() && params.message.task_id.is_none()
        {
            let reply = Message::agent(
                Uuid::new_v4().to_string(),
                vec![Part::text(Self::collect_message_text(&params.message))],
            );
            return match to_value(&reply) {
                Ok(message) => Self::ok(json!({ "message": message }), request.id),
                Err(e) => {
                    error!("Failed to serialize A2A reply message: {e}");
                    Self::a2a_error(-32603, "Failed to serialize reply message", request.id)
                }
            };
        }

        let prepared = match Self::prepare_send(
            resources,
            &user_id,
            params.message,
            request.id.as_ref(),
        )
        .await
        {
            Ok(prepared) => prepared,
            Err(err) => return *err,
        };

        if let Some(config_input) = configuration.push_notification_config {
            if let Err(err) = Self::store_push_config(
                resources,
                &prepared.task_id,
                config_input,
                request.id.as_ref(),
            )
            .await
            {
                return *err;
            }
        }

        let task_id = prepared.task_id.clone(); // Safe: String ownership for post-run lookup

        if configuration.return_immediately {
            // Snapshot first, then execute in the background.
            let snapshot =
                match Self::load_owned_task(resources, &task_id, &user_id, request.id.as_ref())
                    .await
                {
                    Ok(record) => Self::wire_task(&record, configuration.history_length, true),
                    Err(err) => return *err,
                };

            let ctx = resources.ctx.clone(); // Safe: Arc clone for spawned executor
            let tool_runtime = resources.tool_runtime.clone(); // Safe: Arc clone for spawned executor
            let handle = tokio::spawn(async move {
                Self::run_task_message(&ctx, &tool_runtime, user_id, prepared).await;
            });
            drop(handle); // Detached by design: progress is observable via GetTask/SubscribeToTask.

            return Self::send_message_task_response(&snapshot, request.id);
        }

        Self::run_task_message(&resources.ctx, &resources.tool_runtime, user_id, prepared).await;

        match Self::load_owned_task(resources, &task_id, &user_id, request.id.as_ref()).await {
            Ok(record) => {
                let task = Self::wire_task(&record, configuration.history_length, true);
                Self::send_message_task_response(&task, request.id)
            }
            Err(err) => *err,
        }
    }

    /// Resolve the target task for an incoming message (continuation or
    /// creation), persist the user message into its history, and return the
    /// state shared by the blocking/background/streaming paths.
    async fn prepare_send(
        resources: &A2AResources,
        user_id: &Uuid,
        mut message: Message,
        request_id: Option<&Value>,
    ) -> Result<PreparedSend, Box<A2AResponse>> {
        let repos = resources.ctx.repos();

        let (task_id, context_id, mut history) = if let Some(task_id) = message.task_id.clone() {
            // Continuation: the referenced task must exist (client-created
            // task ids are unsupported in A2A 1.0) and belong to the caller.
            let record = Self::load_owned_task(resources, &task_id, user_id, request_id).await?;
            if record.status.is_terminal() {
                return Err(Box::new(Self::spec_error(
                    A2ASpecError::UnsupportedOperation,
                    request_id.cloned(),
                )));
            }
            let record_context = record.context_id.clone().unwrap_or_default();
            if let Some(requested_context) = &message.context_id {
                // A mismatched contextId/taskId pair MUST be rejected.
                if *requested_context != record_context {
                    return Err(Box::new(Self::invalid_params(
                        "contextId does not match the referenced task",
                        request_id.cloned(),
                    )));
                }
            }
            let history = record
                .history
                .as_ref()
                .and_then(|value| from_value::<Vec<Message>>(value.clone()).ok()) // Safe: JSON value ownership for deserialization
                .unwrap_or_default();
            (task_id, record_context, history)
        } else {
            // New task: the server generates ids and returns them.
            let context_id = message
                .context_id
                .clone() // Safe: String ownership for task record
                .unwrap_or_else(|| Uuid::new_v4().to_string());

            let owned = Self::get_owned_client_ids(user_id, resources)
                .await
                .map_err(|e| {
                    error!("Failed to resolve client ownership: {e}");
                    Box::new(Self::a2a_error(
                        -32000,
                        "Failed to resolve client ownership",
                        request_id.cloned(),
                    ))
                })?;
            let Some(client_id) = owned.first() else {
                return Err(Box::new(Self::invalid_params(
                    "No A2A client is registered for this user",
                    request_id.cloned(),
                )));
            };

            let input = to_value(&message).map_err(|e| {
                error!("Failed to serialize A2A message: {e}");
                Box::new(Self::a2a_error(
                    -32603,
                    "Failed to serialize message",
                    request_id.cloned(),
                ))
            })?;

            let task_id = repos
                .a2a
                .create_task(client_id, None, "message", &input, Some(&context_id))
                .await
                .map_err(|e| {
                    error!("Failed to persist task: {e}");
                    Box::new(Self::a2a_error(
                        -32000,
                        "Failed to persist task",
                        request_id.cloned(),
                    ))
                })?;
            info!("Created A2A task {task_id} for client {client_id}");
            (task_id, context_id, Vec::new())
        };

        // Stamp the resolved ids onto the stored user message.
        message.task_id = Some(task_id.clone()); // Safe: String ownership for message
        message.context_id = Some(context_id.clone()); // Safe: String ownership for message
        history.push(message.clone()); // Safe: Message ownership for history

        let history_json = to_value(&history).map_err(|e| {
            error!("Failed to serialize A2A history: {e}");
            Box::new(Self::a2a_error(
                -32603,
                "Failed to serialize history",
                request_id.cloned(),
            ))
        })?;
        repos
            .a2a
            .update_task_wire_state(&task_id, Some(&history_json), None)
            .await
            .map_err(|e| {
                error!("Failed to persist task history: {e}");
                Box::new(Self::a2a_error(
                    -32000,
                    "Failed to persist task history",
                    request_id.cloned(),
                ))
            })?;

        Ok(PreparedSend {
            task_id,
            context_id,
            user_message: message,
            history,
        })
    }

    /// Execute a prepared message against its task: transition to `working`,
    /// run the tool intent (or echo), persist history/artifacts/status,
    /// publish stream events, close the stream, and deliver push
    /// notifications. Infallible by design — failures land on the task as
    /// the `failed` state.
    async fn run_task_message(
        ctx: &Arc<dyn A2ACtx>,
        tool_runtime: &Arc<dyn ToolRuntime>,
        user_id: Uuid,
        prepared: PreparedSend,
    ) {
        let PreparedSend {
            task_id,
            context_id,
            user_message,
            history,
        } = prepared;

        Self::transition_status(ctx, &task_id, &context_id, TaskState::Working, None).await;

        let outcome = match Self::extract_tool_intent(&user_message) {
            Some((tool_name, tool_params)) => {
                Self::execute_registered_tool(ctx, tool_runtime, user_id, &tool_name, tool_params)
                    .await
                    .map_or_else(SendOutcome::Failure, |result| {
                        SendOutcome::Tool(tool_name, result)
                    })
            }
            // No tool intent: acknowledge with a real reply echoing the text.
            None => SendOutcome::Echo,
        };

        match outcome {
            SendOutcome::Failure(failure) => {
                Self::finalize_failed(ctx, &task_id, &context_id, history, failure).await;
            }
            SendOutcome::Tool(tool_name, result) => {
                let artifact = Artifact {
                    artifact_id: Uuid::new_v4().to_string(),
                    parts: vec![Part::data(result.clone())], // Safe: JSON value ownership for part
                    name: Some(format!("{tool_name} result")),
                    description: None,
                    extensions: Vec::new(),
                    metadata: None,
                };
                Self::finalize_completed(
                    ctx,
                    &task_id,
                    &context_id,
                    history,
                    Some(artifact),
                    format!("Tool {tool_name} completed"),
                    Some(result),
                )
                .await;
            }
            SendOutcome::Echo => {
                Self::finalize_completed(
                    ctx,
                    &task_id,
                    &context_id,
                    history,
                    None,
                    Self::collect_message_text(&user_message),
                    None,
                )
                .await;
            }
        }

        // Terminal state reached: end all subscriptions and notify webhooks.
        TASK_EVENTS.close(&task_id);
        Self::notify_webhooks(ctx, &task_id).await;
    }

    /// Build the agent reply for a terminal transition, stamped with the
    /// task/context ids and appended to the history.
    fn build_reply(
        task_id: &str,
        context_id: &str,
        text: String,
        history: &mut Vec<Message>,
    ) -> Message {
        let mut reply = Message::agent(Uuid::new_v4().to_string(), vec![Part::text(text)]);
        reply.task_id = Some(task_id.to_owned());
        reply.context_id = Some(context_id.to_owned());
        history.push(reply.clone()); // Safe: Message ownership for history
        reply
    }

    /// Terminal `failed` transition: explanatory agent reply, persisted
    /// history and status, and a status-update stream event.
    async fn finalize_failed(
        ctx: &Arc<dyn A2ACtx>,
        task_id: &str,
        context_id: &str,
        mut history: Vec<Message>,
        failure: String,
    ) {
        let reply = Self::build_reply(task_id, context_id, failure, &mut history);
        Self::persist_history(ctx, task_id, &history).await;

        let status_message = to_value(&reply).ok();
        if let Err(e) = ctx
            .repos()
            .a2a
            .update_task_status(task_id, &TaskState::Failed, None, status_message.as_ref())
            .await
        {
            error!("Failed to persist A2A task failure: {e}");
        }
        Self::publish_status(task_id, context_id, TaskState::Failed, Some(reply));
    }

    /// Terminal `completed` transition: agent reply, optional artifact with
    /// its stream event, persisted wire state and status, and the final
    /// status-update stream event.
    async fn finalize_completed(
        ctx: &Arc<dyn A2ACtx>,
        task_id: &str,
        context_id: &str,
        mut history: Vec<Message>,
        artifact: Option<Artifact>,
        reply_text: String,
        result: Option<Value>,
    ) {
        let reply = Self::build_reply(task_id, context_id, reply_text, &mut history);

        let artifacts_json = artifact
            .as_ref()
            .and_then(|artifact| to_value(vec![artifact]).ok());
        let history_json = to_value(&history).ok();
        if let Err(e) = ctx
            .repos()
            .a2a
            .update_task_wire_state(task_id, history_json.as_ref(), artifacts_json.as_ref())
            .await
        {
            error!("Failed to persist A2A task wire state: {e}");
        }

        if let Some(artifact) = artifact {
            TASK_EVENTS.publish(
                task_id,
                &StreamResponse::ArtifactUpdate(TaskArtifactUpdateEvent {
                    task_id: task_id.to_owned(),
                    context_id: context_id.to_owned(),
                    artifact,
                    append: false,
                    last_chunk: true,
                    metadata: None,
                }),
            );
        }

        let status_message = to_value(&reply).ok();
        if let Err(e) = ctx
            .repos()
            .a2a
            .update_task_status(
                task_id,
                &TaskState::Completed,
                result.as_ref(),
                status_message.as_ref(),
            )
            .await
        {
            error!("Failed to persist A2A task completion: {e}");
        }
        Self::publish_status(task_id, context_id, TaskState::Completed, Some(reply));
    }

    /// Persist the task's message history, logging (not propagating) failures.
    async fn persist_history(ctx: &Arc<dyn A2ACtx>, task_id: &str, history: &[Message]) {
        if let Ok(history_json) = to_value(history) {
            if let Err(e) = ctx
                .repos()
                .a2a
                .update_task_wire_state(task_id, Some(&history_json), None)
                .await
            {
                error!("Failed to persist A2A task history: {e}");
            }
        }
    }

    /// Persist and publish a non-terminal status transition.
    async fn transition_status(
        ctx: &Arc<dyn A2ACtx>,
        task_id: &str,
        context_id: &str,
        state: TaskState,
        message: Option<Message>,
    ) {
        let status_message = message.as_ref().and_then(|m| to_value(m).ok());
        if let Err(e) = ctx
            .repos()
            .a2a
            .update_task_status(task_id, &state, None, status_message.as_ref())
            .await
        {
            error!("Failed to persist A2A task status transition: {e}");
        }
        Self::publish_status(task_id, context_id, state, message);
    }

    /// Publish a status-update stream event.
    fn publish_status(task_id: &str, context_id: &str, state: TaskState, message: Option<Message>) {
        TASK_EVENTS.publish(
            task_id,
            &StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
                task_id: task_id.to_owned(),
                context_id: context_id.to_owned(),
                status: WireTaskStatus {
                    state,
                    message,
                    timestamp: Some(Self::now_timestamp()),
                },
                metadata: None,
            }),
        );
    }

    /// Deliver the task's current state to its registered webhooks.
    async fn notify_webhooks(ctx: &Arc<dyn A2ACtx>, task_id: &str) {
        let repos = ctx.repos();
        let configs = match repos.a2a.list_push_configs(task_id).await {
            Ok(configs) if !configs.is_empty() => configs,
            Ok(_) => return,
            Err(e) => {
                error!("Failed to load A2A push configs: {e}");
                return;
            }
        };

        let Ok(Some(record)) = repos.a2a.get_task(task_id).await else {
            return;
        };
        let task = Self::wire_task(&record, None, true);
        let event = StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
            task_id: task.id.clone(), // Safe: String ownership for event
            context_id: task.context_id.clone().unwrap_or_default(), // Safe: String ownership for event
            status: task.status,
            metadata: None,
        });
        push::deliver_task_update(&configs, &event).await;
    }

    /// Extract a `(tool_name, parameters)` tool invocation from a message:
    /// a `data` part carrying `{"tool_name": ..., "parameters": {...}}`.
    #[must_use]
    pub fn extract_tool_intent(message: &Message) -> Option<(String, Value)> {
        for part in &message.parts {
            let PartContent::Data(content) = &part.content else {
                continue;
            };
            if let Some(tool_name) = content.get("tool_name").and_then(Value::as_str) {
                let tool_params = content
                    .get("parameters")
                    .cloned() // Safe: JSON value ownership for execution
                    .unwrap_or_else(|| Value::Object(Map::default()));
                return Some((tool_name.to_owned(), tool_params));
            }
        }
        None
    }

    /// Concatenate the text content of all `text` parts of a message.
    #[must_use]
    pub fn collect_message_text(message: &Message) -> String {
        message
            .parts
            .iter()
            .filter_map(|part| match &part.content {
                PartContent::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Execute a registered tool against the authenticated user's tenant.
    /// Returns the tool output on success, or a human-readable failure
    /// description (surfaced on the task as the `failed` status message).
    async fn execute_registered_tool(
        ctx: &Arc<dyn A2ACtx>,
        tool_runtime: &Arc<dyn ToolRuntime>,
        user_id: Uuid,
        tool_name: &str,
        tool_params: Value,
    ) -> Result<Value, String> {
        // Resolve tenant context — required for all A2A tool execution.
        let tenant_context =
            match extract_tenant_context_internal(ctx.repos(), Some(user_id), None, None).await {
                Ok(Some(context)) => context,
                Ok(None) => return Err("User does not belong to any tenant".into()),
                Err(e) => {
                    error!("Failed to resolve tenant context: {e}");
                    return Err("Failed to resolve tenant context".into());
                }
            };

        if !tool_runtime.tool_registry().contains(tool_name) {
            return Err(format!("Unknown tool: {tool_name}"));
        }

        // Route through the unified executor so A2A tool calls pass the Guardian
        // chokepoint and resolve admin/tenant context exactly like the MCP and
        // chat paths.
        let executor = UniversalToolExecutor::new(tool_runtime.clone()); // Safe: Arc clone for executor
        let request = UniversalRequest {
            tool_name: tool_name.to_owned(),
            parameters: tool_params,
            user_id: user_id.to_string(),
            protocol: "a2a".to_owned(),
            tenant_id: Some(tenant_context.tenant_id.to_string()),
            progress_token: None,
            cancellation_token: None,
            progress_reporter: None,
        };

        match executor.execute_tool(request).await {
            Ok(response) if response.success => Ok(response
                .result
                .unwrap_or_else(|| Value::Object(Map::default()))),
            Ok(response) => Err(response
                .error
                .unwrap_or_else(|| "Tool execution failed".into())),
            Err(e) => Err(e.to_string()),
        }
    }

    // ============================ Streaming entry ============================

    /// Start a `SendStreamingMessage` exchange: prepare the task, subscribe
    /// to its event stream, kick off background execution, and return the
    /// initial task snapshot plus the live receiver. The HTTP layer frames
    /// the snapshot and subsequent events as SSE.
    ///
    /// # Errors
    ///
    /// Returns the JSON-RPC error response when authentication, parameter
    /// validation, or task preparation fails.
    pub async fn start_streaming_message(
        &self,
        request: A2ARequest,
    ) -> Result<(Task, broadcast::Receiver<StreamResponse>), Box<A2AResponse>> {
        let Some(resources) = &self.resources else {
            return Err(Box::new(Self::server_not_configured_error(request.id)));
        };
        let user_id = Self::authenticate_request(&request, resources)?;

        let params: SendMessageRequest = match request
            .params
            .clone() // Safe: JSON value ownership for deserialization
            .map(from_value)
            .transpose()
        {
            Ok(Some(params)) => params,
            Ok(None) => {
                return Err(Box::new(Self::invalid_params(
                    "Missing params: message",
                    request.id,
                )))
            }
            Err(e) => {
                return Err(Box::new(Self::invalid_params(
                    format!("Invalid params: {e}"),
                    request.id,
                )))
            }
        };

        if params.message.parts.is_empty() {
            return Err(Box::new(Self::invalid_params(
                "message.parts must not be empty",
                request.id,
            )));
        }

        let configuration = params.configuration.unwrap_or_default();

        let prepared =
            Self::prepare_send(resources, &user_id, params.message, request.id.as_ref()).await?;

        if let Some(config_input) = configuration.push_notification_config {
            Self::store_push_config(
                resources,
                &prepared.task_id,
                config_input,
                request.id.as_ref(),
            )
            .await?;
        }

        // Subscribe before spawning so no event can be missed.
        let receiver = TASK_EVENTS.subscribe(&prepared.task_id);

        let snapshot = match Self::load_owned_task(
            resources,
            &prepared.task_id,
            &user_id,
            request.id.as_ref(),
        )
        .await
        {
            Ok(record) => Self::wire_task(&record, configuration.history_length, true),
            Err(err) => return Err(err),
        };

        let ctx = resources.ctx.clone(); // Safe: Arc clone for spawned executor
        let tool_runtime = resources.tool_runtime.clone(); // Safe: Arc clone for spawned executor
        let handle = tokio::spawn(async move {
            Self::run_task_message(&ctx, &tool_runtime, user_id, prepared).await;
        });
        drop(handle); // Detached by design: completion is signalled by stream closure.

        Ok((snapshot, receiver))
    }

    /// Start a `SubscribeToTask` stream: validate access, reject terminal
    /// tasks, and return the current snapshot plus the live receiver. The
    /// snapshot MUST be the first SSE frame (spec §9.4.2).
    ///
    /// # Errors
    ///
    /// Returns the JSON-RPC error response when authentication fails, the
    /// task is unknown, or it is already terminal (`UnsupportedOperation`).
    pub async fn start_task_subscription(
        &self,
        request: A2ARequest,
    ) -> Result<(Task, broadcast::Receiver<StreamResponse>), Box<A2AResponse>> {
        let Some(resources) = &self.resources else {
            return Err(Box::new(Self::server_not_configured_error(request.id)));
        };
        let user_id = Self::authenticate_request(&request, resources)?;

        let params: SubscribeParams = match request
            .params
            .clone() // Safe: JSON value ownership for deserialization
            .map(from_value)
            .transpose()
        {
            Ok(Some(params)) => params,
            Ok(None) => {
                return Err(Box::new(Self::invalid_params(
                    "Missing params: id",
                    request.id,
                )))
            }
            Err(e) => {
                return Err(Box::new(Self::invalid_params(
                    format!("Invalid params: {e}"),
                    request.id,
                )))
            }
        };

        let record =
            Self::load_owned_task(resources, &params.id, &user_id, request.id.as_ref()).await?;

        if record.status.is_terminal() {
            return Err(Box::new(Self::spec_error(
                A2ASpecError::UnsupportedOperation,
                request.id,
            )));
        }

        let receiver = TASK_EVENTS.subscribe(&record.id);
        let snapshot = Self::wire_task(&record, params.history_length, true);
        Ok((snapshot, receiver))
    }

    // ============================== Task queries =============================

    async fn handle_get_task(&self, request: A2ARequest, user_id: Uuid) -> A2AResponse {
        let Some(resources) = &self.resources else {
            return Self::server_not_configured_error(request.id);
        };

        let params: GetTaskRequest = match request
            .params
            .clone() // Safe: JSON value ownership for deserialization
            .map(from_value)
            .transpose()
        {
            Ok(Some(params)) => params,
            Ok(None) => return Self::invalid_params("Missing params: id", request.id),
            Err(e) => return Self::invalid_params(format!("Invalid params: {e}"), request.id),
        };

        match Self::load_owned_task(resources, &params.id, &user_id, request.id.as_ref()).await {
            Ok(record) => {
                let task = Self::wire_task(&record, params.history_length, true);
                Self::task_response(&task, request.id)
            }
            Err(err) => *err,
        }
    }

    /// Parse and validate `ListTasks` params into a [`ListTasksQuery`].
    fn parse_list_tasks_params(request: &A2ARequest) -> Result<ListTasksQuery, Box<A2AResponse>> {
        let params: ListTasksRequest = match request
            .params
            .clone() // Safe: JSON value ownership for deserialization
            .map(from_value)
            .transpose()
        {
            Ok(params) => params.unwrap_or_default(),
            Err(e) => {
                return Err(Box::new(Self::invalid_params(
                    format!("Invalid params: {e}"),
                    request.id.clone(),
                )))
            }
        };

        let page_size = params
            .page_size
            .unwrap_or(LIST_TASKS_DEFAULT_PAGE_SIZE)
            .clamp(1, LIST_TASKS_MAX_PAGE_SIZE);

        let offset = match params.page_token.as_deref() {
            None | Some("") => 0_u32,
            Some(token) => match token.strip_prefix('o').and_then(|n| n.parse::<u32>().ok()) {
                Some(offset) => offset,
                None => {
                    return Err(Box::new(Self::invalid_params(
                        "Invalid pageToken",
                        request.id.clone(),
                    )))
                }
            },
        };

        let updated_after = match params.status_timestamp_after.as_deref() {
            None => None,
            Some(ts) => match chrono::DateTime::parse_from_rfc3339(ts) {
                Ok(instant) => Some(instant.with_timezone(&chrono::Utc)),
                Err(_) => {
                    return Err(Box::new(Self::invalid_params(
                        "Invalid statusTimestampAfter: expected ISO-8601",
                        request.id.clone(),
                    )))
                }
            },
        };

        Ok(ListTasksQuery {
            params,
            page_size,
            offset,
            updated_after,
        })
    }

    /// Fetch the caller's task records across every owned client (usually a
    /// single client), merged and sorted by status timestamp descending.
    async fn fetch_owned_task_records(
        resources: &A2AResources,
        user_id: &Uuid,
        params: &ListTasksRequest,
        updated_after: Option<chrono::DateTime<chrono::Utc>>,
        fetch_limit: u32,
        request_id: Option<&Value>,
    ) -> Result<Vec<A2ATask>, Box<A2AResponse>> {
        let owned_client_ids = Self::get_owned_client_ids(user_id, resources)
            .await
            .map_err(|e| {
                error!("Failed to resolve client ownership: {e}");
                Box::new(Self::a2a_error(
                    -32000,
                    "Failed to resolve client ownership",
                    request_id.cloned(),
                ))
            })?;

        let mut records: Vec<A2ATask> = Vec::new();
        for client_id in &owned_client_ids {
            let tasks = resources
                .ctx
                .repos()
                .a2a
                .list_tasks(
                    Some(client_id),
                    params.status.as_ref(),
                    params.context_id.as_deref(),
                    updated_after,
                    Some(fetch_limit),
                    None,
                )
                .await
                .map_err(|e| {
                    error!("A2A task list database error: {e}");
                    Box::new(Self::a2a_error(
                        -32000,
                        "Database error",
                        request_id.cloned(),
                    ))
                })?;
            records.extend(tasks);
        }
        records.sort_by_key(|record| Reverse(record.updated_at));
        Ok(records)
    }

    async fn handle_list_tasks(&self, request: A2ARequest, user_id: Uuid) -> A2AResponse {
        let Some(resources) = &self.resources else {
            return Self::server_not_configured_error(request.id);
        };

        let query = match Self::parse_list_tasks_params(&request) {
            Ok(query) => query,
            Err(err) => return *err,
        };
        let ListTasksQuery {
            params,
            page_size,
            offset,
            updated_after,
        } = query;

        // Fetch one page-plus-one rows so the continuation token is exact.
        let fetch_limit = offset.saturating_add(page_size).saturating_add(1);
        let records = match Self::fetch_owned_task_records(
            resources,
            &user_id,
            &params,
            updated_after,
            fetch_limit,
            request.id.as_ref(),
        )
        .await
        {
            Ok(records) => records,
            Err(err) => return *err,
        };

        let skip_count = usize::try_from(offset).unwrap_or(usize::MAX);
        let take_count = usize::try_from(page_size).unwrap_or(usize::MAX);
        let has_more = records.len() > skip_count.saturating_add(take_count);

        let tasks: Vec<Task> = records
            .iter()
            .skip(skip_count)
            .take(take_count)
            .map(|record| Self::wire_task(record, params.history_length, params.include_artifacts))
            .collect();

        let next_page_token = if has_more {
            format!("o{}", offset.saturating_add(page_size))
        } else {
            String::new()
        };

        let response = ListTasksResponse {
            tasks,
            next_page_token,
        };
        match to_value(&response) {
            Ok(value) => Self::ok(value, request.id),
            Err(e) => {
                error!("Failed to serialize A2A task list: {e}");
                Self::a2a_error(-32603, "Failed to serialize task list", request.id)
            }
        }
    }

    async fn handle_cancel_task(&self, request: A2ARequest, user_id: Uuid) -> A2AResponse {
        let Some(resources) = &self.resources else {
            return Self::server_not_configured_error(request.id);
        };

        let params: CancelParams = match request
            .params
            .clone() // Safe: JSON value ownership for deserialization
            .map(from_value)
            .transpose()
        {
            Ok(Some(params)) => params,
            Ok(None) => return Self::invalid_params("Missing params: id", request.id),
            Err(e) => return Self::invalid_params(format!("Invalid params: {e}"), request.id),
        };

        let record =
            match Self::load_owned_task(resources, &params.id, &user_id, request.id.as_ref()).await
            {
                Ok(record) => record,
                Err(err) => return *err,
            };

        if record.status.is_terminal() {
            return Self::spec_error(A2ASpecError::TaskNotCancelable, request.id);
        }

        if let Err(e) = resources
            .ctx
            .repos()
            .a2a
            .update_task_status(&record.id, &TaskState::Canceled, None, None)
            .await
        {
            error!("Failed to cancel A2A task: {e}");
            return Self::a2a_error(-32000, "Failed to cancel task", request.id);
        }

        let context_id = record.context_id.clone().unwrap_or_default();
        Self::publish_status(&record.id, &context_id, TaskState::Canceled, None);
        TASK_EVENTS.close(&record.id);
        Self::notify_webhooks(&resources.ctx, &record.id).await;

        match Self::load_owned_task(resources, &record.id, &user_id, request.id.as_ref()).await {
            Ok(updated) => {
                let task = Self::wire_task(&updated, None, true);
                Self::task_response(&task, request.id)
            }
            Err(err) => *err,
        }
    }

    // ========================= Push notification CRUD ========================

    /// Validate and persist a push notification configuration on a task.
    async fn store_push_config(
        resources: &A2AResources,
        task_id: &str,
        input: PushNotificationConfigInput,
        request_id: Option<&Value>,
    ) -> Result<TaskPushNotificationConfig, Box<A2AResponse>> {
        if let Err(reason) = push::validate_webhook_url(&input.url).await {
            return Err(Box::new(Self::invalid_params(
                format!("Invalid push notification URL: {reason}"),
                request_id.cloned(),
            )));
        }

        let now = chrono::Utc::now();
        let record = A2APushNotificationConfig {
            config_id: input.id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            task_id: task_id.to_owned(),
            url: input.url,
            token: input.token,
            auth_scheme: input.authentication.as_ref().map(|a| a.scheme.clone()), // Safe: String ownership for record
            auth_credentials: input
                .authentication
                .as_ref()
                .and_then(|a| a.credentials.clone()), // Safe: String ownership for record
            created_at: now,
            updated_at: now,
        };

        resources
            .ctx
            .repos()
            .a2a
            .create_push_config(&record)
            .await
            .map_err(|e| {
                error!("Failed to persist A2A push config: {e}");
                Box::new(Self::a2a_error(
                    -32000,
                    "Failed to persist push notification config",
                    request_id.cloned(),
                ))
            })?;

        Ok(Self::wire_push_config(&record))
    }

    /// Convert a stored push config into its wire shape.
    fn wire_push_config(record: &A2APushNotificationConfig) -> TaskPushNotificationConfig {
        TaskPushNotificationConfig {
            id: record.config_id.clone(), // Safe: String ownership for wire object
            task_id: record.task_id.clone(), // Safe: String ownership for wire object
            url: record.url.clone(),      // Safe: String ownership for wire object
            token: record.token.clone(),  // Safe: String ownership for wire object
            authentication: record.auth_scheme.as_ref().map(|scheme| {
                PushNotificationAuthenticationInfo {
                    scheme: scheme.clone(), // Safe: String ownership for wire object
                    credentials: record.auth_credentials.clone(), // Safe: String ownership for wire object
                }
            }),
        }
    }

    async fn handle_create_push_config(&self, request: A2ARequest, user_id: Uuid) -> A2AResponse {
        let Some(resources) = &self.resources else {
            return Self::server_not_configured_error(request.id);
        };

        let params: CreatePushConfigParams = match request
            .params
            .clone() // Safe: JSON value ownership for deserialization
            .map(from_value)
            .transpose()
        {
            Ok(Some(params)) => params,
            Ok(None) => return Self::invalid_params("Missing params: taskId, config", request.id),
            Err(e) => return Self::invalid_params(format!("Invalid params: {e}"), request.id),
        };

        if let Err(err) =
            Self::load_owned_task(resources, &params.task_id, &user_id, request.id.as_ref()).await
        {
            return *err;
        }

        match Self::store_push_config(
            resources,
            &params.task_id,
            params.config,
            request.id.as_ref(),
        )
        .await
        {
            Ok(config) => match to_value(&config) {
                Ok(value) => Self::ok(value, request.id),
                Err(e) => {
                    error!("Failed to serialize A2A push config: {e}");
                    Self::a2a_error(-32603, "Failed to serialize push config", request.id)
                }
            },
            Err(err) => *err,
        }
    }

    async fn handle_get_push_config(&self, request: A2ARequest, user_id: Uuid) -> A2AResponse {
        let Some(resources) = &self.resources else {
            return Self::server_not_configured_error(request.id);
        };

        let params: PushConfigRefParams = match request
            .params
            .clone() // Safe: JSON value ownership for deserialization
            .map(from_value)
            .transpose()
        {
            Ok(Some(params)) => params,
            Ok(None) => {
                return Self::invalid_params("Missing params: taskId, configId", request.id)
            }
            Err(e) => return Self::invalid_params(format!("Invalid params: {e}"), request.id),
        };

        if let Err(err) =
            Self::load_owned_task(resources, &params.task_id, &user_id, request.id.as_ref()).await
        {
            return *err;
        }

        match resources
            .ctx
            .repos()
            .a2a
            .get_push_config(&params.task_id, &params.config_id)
            .await
        {
            Ok(Some(record)) => match to_value(Self::wire_push_config(&record)) {
                Ok(value) => Self::ok(value, request.id),
                Err(e) => {
                    error!("Failed to serialize A2A push config: {e}");
                    Self::a2a_error(-32603, "Failed to serialize push config", request.id)
                }
            },
            Ok(None) => Self::invalid_params("Unknown push notification config id", request.id),
            Err(e) => {
                error!("Failed to load A2A push config: {e}");
                Self::a2a_error(-32000, "Database error", request.id)
            }
        }
    }

    async fn handle_list_push_configs(&self, request: A2ARequest, user_id: Uuid) -> A2AResponse {
        let Some(resources) = &self.resources else {
            return Self::server_not_configured_error(request.id);
        };

        let params: PushConfigListParams = match request
            .params
            .clone() // Safe: JSON value ownership for deserialization
            .map(from_value)
            .transpose()
        {
            Ok(Some(params)) => params,
            Ok(None) => return Self::invalid_params("Missing params: taskId", request.id),
            Err(e) => return Self::invalid_params(format!("Invalid params: {e}"), request.id),
        };

        if let Err(err) =
            Self::load_owned_task(resources, &params.task_id, &user_id, request.id.as_ref()).await
        {
            return *err;
        }

        match resources
            .ctx
            .repos()
            .a2a
            .list_push_configs(&params.task_id)
            .await
        {
            Ok(records) => {
                let configs: Vec<TaskPushNotificationConfig> =
                    records.iter().map(Self::wire_push_config).collect();
                Self::ok(json!({ "configs": configs }), request.id)
            }
            Err(e) => {
                error!("Failed to list A2A push configs: {e}");
                Self::a2a_error(-32000, "Database error", request.id)
            }
        }
    }

    async fn handle_delete_push_config(&self, request: A2ARequest, user_id: Uuid) -> A2AResponse {
        let Some(resources) = &self.resources else {
            return Self::server_not_configured_error(request.id);
        };

        let params: PushConfigRefParams = match request
            .params
            .clone() // Safe: JSON value ownership for deserialization
            .map(from_value)
            .transpose()
        {
            Ok(Some(params)) => params,
            Ok(None) => {
                return Self::invalid_params("Missing params: taskId, configId", request.id)
            }
            Err(e) => return Self::invalid_params(format!("Invalid params: {e}"), request.id),
        };

        if let Err(err) =
            Self::load_owned_task(resources, &params.task_id, &user_id, request.id.as_ref()).await
        {
            return *err;
        }

        match resources
            .ctx
            .repos()
            .a2a
            .delete_push_config(&params.task_id, &params.config_id)
            .await
        {
            Ok(true) => Self::ok(Value::Object(Map::default()), request.id),
            Ok(false) => Self::invalid_params("Unknown push notification config id", request.id),
            Err(e) => {
                error!("Failed to delete A2A push config: {e}");
                Self::a2a_error(-32000, "Database error", request.id)
            }
        }
    }
}

impl Default for A2AServer {
    fn default() -> Self {
        Self::new()
    }
}

/// `SubscribeToTask` params (`id` + optional `historyLength` for the snapshot).
#[derive(serde::Deserialize)]
struct SubscribeParams {
    /// Task identifier
    id: String,
    /// History truncation for the initial snapshot
    #[serde(rename = "historyLength", default)]
    history_length: Option<u32>,
}

/// `CancelTask` params.
#[derive(serde::Deserialize)]
struct CancelParams {
    /// Task identifier
    id: String,
}

/// `CreateTaskPushNotificationConfig` params.
#[derive(serde::Deserialize)]
struct CreatePushConfigParams {
    /// Task the config attaches to
    #[serde(rename = "taskId")]
    task_id: String,
    /// The configuration to register
    config: PushNotificationConfigInput,
}

/// `GetTaskPushNotificationConfig` / `DeleteTaskPushNotificationConfig` params.
#[derive(serde::Deserialize)]
struct PushConfigRefParams {
    /// Task the config attaches to
    #[serde(rename = "taskId")]
    task_id: String,
    /// The configuration identifier
    #[serde(rename = "configId")]
    config_id: String,
}

/// `ListTaskPushNotificationConfigs` params.
#[derive(serde::Deserialize)]
struct PushConfigListParams {
    /// Task the configs attach to
    #[serde(rename = "taskId")]
    task_id: String,
}
