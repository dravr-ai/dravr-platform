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

use crate::coaching::CoachingTurns;
use crate::events::TASK_EVENTS;
use crate::protocol_types::{
    A2ASpecError, Artifact, Message, Part, PartContent, SendMessageRequest, StreamResponse, Task,
    TaskState, WireTaskStatus,
};
use crate::{A2ARequest, A2AResponse};
use dravr_tronc::dates::SecondsFormat;
use dravr_tronc::error::{INTERNAL_ERROR, INVALID_REQUEST, METHOD_NOT_FOUND, SERVER_ERROR_START};
use pierre_core::models::a2a::user_task_key;
pub use pierre_core::models::a2a::{A2ATask, TaskStatus};
use pierre_core::permissions::impersonation::CallerCredential;
use pierre_core::transport::Transport;
use pierre_mcp_transport::tenant_isolation::{extract_tenant_context_internal, log_tenant_failure};
use pierre_middleware::McpAuthMiddleware;
use pierre_runtime_context::A2ACtx;
use pierre_tool_runtime::context::SUPPORT_SESSION;
use pierre_tool_runtime::protocol::trust_labels::{self, TRUST_LABELS_META_KEY};
use pierre_tool_runtime::protocol::{
    grant_block, guardian_block_reason, record_refused_admin_call, RefusedAdminCall,
    UniversalRequest, UniversalResponse, UniversalToolExecutor,
};
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::support_session::{self, CALLER_CREDENTIAL};
use std::fmt;
// Trait methods dispatched through repos.a2a Arc<dyn Trait>;
use serde_json::{from_value, json, to_value, Map, Value};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{error, info};
use uuid::Uuid;

/// The JSON-RPC and A2A-spec error responses every handler answers with
mod errors;
/// `GetExtendedAgentCard`: the live registry's tools as skills, listed per caller
mod extended_card;
mod finalize;
mod params;
mod principal;
mod push_config;
/// An admin's support session through their agent, as A2A serves it (carnet#1511)
mod support;
mod task_members;
mod task_reads;
mod tasks;
mod text_turn;
use principal::AuthPrincipal;
use text_turn::AdmittedTextTurn;

use dravr_tronc::dates::Timestamp;
use params::SubscribeParams;

/// Caller-facing explanation for a message that holds nothing to run: no text
/// for the coach, and no data part naming a registered tool.
const NO_TOOL_INTENT_MESSAGE: &str =
    "A2A messages must carry a text part for the coach, or address a registered tool with a \
     data part shaped {\"tool_name\": \"<tool>\", \"parameters\": {...}}.";

/// Bundle of runtime handles required for full A2A protocol operation.
///
/// [`A2ACtx`] supplies the auth/repos/base-url slice; `tool_runtime` and
/// `auth_middleware` are held separately to avoid forcing
/// `pierre-runtime-context` to depend on `pierre-tool-runtime` and
/// `pierre-middleware`. The bundle keeps `A2AServer`'s field count manageable
/// and groups the handles that must always be wired together when the server
/// runs against real infrastructure.
pub struct A2AResources {
    /// Narrow runtime-context slice: auth manager, JWKS, repos, base URL.
    pub ctx: Arc<dyn A2ACtx>,
    /// Tool runtime façade — dispatches `SendMessage` tool intents into the
    /// shared `ToolRegistry`.
    pub tool_runtime: Arc<dyn ToolRuntime>,
    /// The auth pipeline every other route admits a user credential through:
    /// account status, request budget, usage row and budget report.
    pub auth_middleware: Arc<McpAuthMiddleware>,
    /// The coaching pipeline a text message runs in. `None` in a build without
    /// it, where a text message is refused as a message with nothing to run.
    pub coaching: Option<Arc<dyn CoachingTurns>>,
}

impl fmt::Debug for A2AResources {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("A2AResources")
            .field("auth_middleware", &self.auth_middleware)
            .field("coaching", &self.coaching.is_some())
            .finish_non_exhaustive()
    }
}

/// A2A Protocol Server implementation
#[derive(Debug)]
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
    /// The coaching turn the message was admitted to, when it is a text
    /// message; `None` runs the tool intent.
    text_turn: Option<AdmittedTextTurn>,
}

/// Why a message produced no tool output.
struct SendFailure {
    /// The human-readable reason, surfaced as the `failed` status message.
    reason: String,
    /// The Guardian's machine-readable refusal when it stopped the call
    /// (`error_code` + `reason`), for a client to branch on. It rides beside the
    /// text as a data part, so a client reading only the text sees what it did.
    data: Option<Value>,
}

impl SendFailure {
    /// A failure with nothing beyond its text.
    fn text(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            data: None,
        }
    }
}

/// Result of executing a message against its task.
enum SendOutcome {
    /// A named tool ran successfully and produced this result, with the
    /// trust label the executor put on it, if any
    Tool(String, Value, Option<Value>),
    /// The coach answered the message
    Coach(String),
    /// The caller canceled the task while the coach was answering: the task
    /// already stands where they put it
    Canceled,
    /// The message produced no tool output
    Failure(SendFailure),
}

/// The Guardian's `error_code` and `reason` when it denied the call, and the
/// consent grant's (its ceiling, its daily row budget with `retry_after_secs`),
/// read only from a response the chokepoint itself stamped: a tool cannot put
/// words here.
fn guardian_refusal_data(response: &UniversalResponse) -> Option<Value> {
    (guardian_block_reason(response) == Some("guardian") || grant_block(response))
        .then(|| response.result.clone())
        .flatten()
}

impl A2AServer {
    /// Create a new A2A server without resources
    #[must_use]
    pub const fn new() -> Self {
        Self { resources: None }
    }

    /// Create a new A2A server with server resources
    #[must_use]
    pub fn new_with_resources(
        ctx: Arc<dyn A2ACtx>,
        tool_runtime: Arc<dyn ToolRuntime>,
        auth_middleware: Arc<McpAuthMiddleware>,
        coaching: Option<Arc<dyn CoachingTurns>>,
    ) -> Self {
        Self {
            resources: Some(A2AResources {
                ctx,
                tool_runtime,
                auth_middleware,
                coaching,
            }),
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
                self.require_auth_then(request, |s, req, principal| {
                    Box::pin(s.handle_send_message(req, principal))
                })
                .await
            }
            "GetTask" => {
                self.require_auth_then(request, |s, req, principal| {
                    Box::pin(s.handle_get_task(req, principal))
                })
                .await
            }
            "ListTasks" => {
                self.require_auth_then(request, |s, req, principal| {
                    Box::pin(s.handle_list_tasks(req, principal))
                })
                .await
            }
            "CancelTask" => {
                self.require_auth_then(request, |s, req, principal| {
                    Box::pin(s.handle_cancel_task(req, principal))
                })
                .await
            }
            "CreateTaskPushNotificationConfig" => {
                self.require_auth_then(request, |s, req, principal| {
                    Box::pin(s.handle_create_push_config(req, principal))
                })
                .await
            }
            "GetTaskPushNotificationConfig" => {
                self.require_auth_then(request, |s, req, principal| {
                    Box::pin(s.handle_get_push_config(req, principal))
                })
                .await
            }
            "ListTaskPushNotificationConfigs" => {
                self.require_auth_then(request, |s, req, principal| {
                    Box::pin(s.handle_list_push_configs(req, principal))
                })
                .await
            }
            "DeleteTaskPushNotificationConfig" => {
                self.require_auth_then(request, |s, req, principal| {
                    Box::pin(s.handle_delete_push_config(req, principal))
                })
                .await
            }
            // The authenticated extended card: the public card enriched with
            // the live tool registry as skills.
            "GetExtendedAgentCard" => {
                self.require_auth_then(request, |s, req, principal| {
                    Box::pin(s.handle_extended_card(req, principal))
                })
                .await
            }
            // SSE-only methods: the HTTP layer intercepts these before
            // non-streaming dispatch; a non-SSE caller lands here.
            "SendStreamingMessage" | "SubscribeToTask" => Self::a2a_error(
                INVALID_REQUEST,
                "Streaming methods require the SSE transport",
                request.id,
            ),
            _ => Self::a2a_error(
                METHOD_NOT_FOUND,
                format!("Method not found: {}", request.method),
                request.id,
            ),
        }
    }

    /// Require authentication before dispatching to a handler
    ///
    /// Extracts and validates the JWT auth token from the request, then calls
    /// the handler with the authenticated principal. Returns an auth error
    /// response if authentication fails.
    async fn require_auth_then<F>(&self, request: A2ARequest, handler: F) -> A2AResponse
    where
        F: FnOnce(
            &Self,
            A2ARequest,
            AuthPrincipal,
        ) -> Pin<Box<dyn Future<Output = A2AResponse> + Send + '_>>,
    {
        let Some(resources) = &self.resources else {
            return Self::server_not_configured_error(request.id.clone());
        };
        match Self::authenticate_request(&request, resources).await {
            Ok(principal) => handler(self, request, principal).await,
            Err(err_response) => *err_response,
        }
    }

    /// Successful JSON-RPC response wrapper.
    fn ok(result: Value, request_id: Option<Value>) -> A2AResponse {
        A2AResponse::success(request_id, result)
    }

    /// Current instant in the spec's ISO-8601 UTC millisecond format.
    pub(crate) fn now_timestamp() -> String {
        Timestamp::now().to_rfc3339_opts(SecondsFormat::Millis, true)
    }

    /// Assemble the wire `Task` from a database record, applying
    /// `historyLength` truncation and artifact inclusion.
    pub(crate) fn wire_task(
        record: &A2ATask,
        history_length: Option<u32>,
        include_artifacts: bool,
    ) -> Task {
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
                Self::a2a_error(INTERNAL_ERROR, "Failed to serialize task", request_id)
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
                Self::a2a_error(INTERNAL_ERROR, "Failed to serialize task", request_id)
            }
        }
    }

    // ============================== SendMessage ==============================

    /// Handle A2A 1.0 `SendMessage`.
    ///
    /// The surface accepts two intents. **Tool invocation**: when the message
    /// carries a `data` part with `{"tool_name": ..., "parameters": {...}}`,
    /// the named tool executes against the authenticated user's tenant
    /// through the shared registry path; the output lands on the task as an
    /// artifact. **A coaching turn**: a `text` part addressing no tool runs
    /// one turn of the coaching pipeline for the athlete the token acts for
    /// ([`text_turn`]); the reply lands on the task as text.
    ///
    /// A message holding neither is refused with the spec's
    /// `ContentTypeNotSupported` (`-32005`) error naming the accepted shapes;
    /// when it references an existing task, the task transitions to
    /// `failed` carrying the same explanation.
    ///
    /// With `configuration.returnImmediately == false` (the default) the
    /// call blocks until the task reaches a terminal state and returns the
    /// final task snapshot; with `true` it returns the submitted snapshot
    /// and execution continues in the background. A coaching turn blocks
    /// detached from the request, under a budget inside the HTTP door's 600 s,
    /// so a client that drops the connection neither kills the turn nor leaves
    /// its task `working`.
    async fn handle_send_message(
        &self,
        request: A2ARequest,
        principal: AuthPrincipal,
    ) -> A2AResponse {
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
        let mut message = params.message;

        let text_turn =
            match Self::admit_text_turn(resources, &principal, &mut message, request.id.as_ref())
                .await
            {
                Ok(text_turn) => text_turn,
                Err(err) => return *err,
            };

        // A message that holds no text for the coach, addresses no registered
        // tool and references no task has nothing to execute: reject it
        // instead of opening a task that cannot make progress.
        if text_turn.is_none()
            && Self::extract_tool_intent(&message).is_none()
            && message.task_id.is_none()
        {
            return Self::spec_error_with_message(
                A2ASpecError::ContentTypeNotSupported,
                NO_TOOL_INTENT_MESSAGE,
                request.id,
            );
        }

        let tool = Self::extract_tool_intent(&message).map(|(tool, _)| tool);
        let mut prepared = match Self::prepare_send(
            resources,
            &principal,
            message,
            text_turn.is_some(),
            request.id.as_ref(),
        )
        .await
        {
            Ok(prepared) => prepared,
            Err(err) => {
                Self::record_refused_send(resources, &principal, tool.as_deref()).await;
                return *err;
            }
        };
        prepared.text_turn = text_turn;

        if let Some(config_input) = configuration.push_notification_config {
            if let Err(err) = Self::store_push_config(
                resources,
                &prepared.task_id,
                config_input,
                request.id.as_ref(),
            )
            .await
            {
                Self::record_refused_send(resources, &principal, tool.as_deref()).await;
                return *err;
            }
        }

        let task_id = prepared.task_id.clone(); // Safe: String ownership for post-run lookup

        if configuration.return_immediately {
            // Snapshot first, then execute in the background.
            let snapshot =
                match Self::load_owned_task(resources, &task_id, &principal, request.id.as_ref())
                    .await
                {
                    Ok(record) => Self::wire_task(&record, configuration.history_length, true),
                    Err(err) => return *err,
                };

            let ctx = resources.ctx.clone(); // Safe: Arc clone for spawned executor
            let tool_runtime = resources.tool_runtime.clone(); // Safe: Arc clone for spawned executor
                                                               // Detached by design: progress is observable via GetTask/SubscribeToTask.
                                                               // The context spawns it under the server's in-flight tracker so a
                                                               // drain waits for it; a task the drain still loses is failed by
                                                               // the reaper (`crate::reaper`).
            let spawner = Arc::clone(&resources.ctx);
            spawner.spawn_detached_task(Box::pin(async move {
                Self::run_task_message(&ctx, &tool_runtime, principal, prepared).await;
            }));

            return Self::send_message_task_response(&snapshot, request.id);
        }

        let (ctx, runtime) = (&resources.ctx, &resources.tool_runtime);
        if prepared.text_turn.is_some() {
            Self::run_detached_and_wait(ctx, runtime, principal.clone(), prepared).await;
        } else {
            Self::run_task_message(ctx, runtime, principal.clone(), prepared).await;
        }

        match Self::load_owned_task(resources, &task_id, &principal, request.id.as_ref()).await {
            Ok(record) => {
                let task = Self::wire_task(&record, configuration.history_length, true);
                Self::send_message_task_response(&task, request.id)
            }
            Err(err) => *err,
        }
    }

    /// The client a new task is keyed to: client-credentials callers key
    /// tasks to themselves; user JWTs fall back to the user's first
    /// registered client, or to the user's own task key when they registered
    /// none, as an athlete who connected an agent over OAuth has not.
    async fn resolve_task_client(
        principal: &AuthPrincipal,
        resources: &A2AResources,
        request_id: Option<&Value>,
    ) -> Result<String, Box<A2AResponse>> {
        if let Some(acting_client) = &principal.client_id {
            return Ok(acting_client.clone()); // Safe: String ownership for task record
        }

        let owned = Self::get_owned_client_ids(&principal.user_id, resources)
            .await
            .map_err(|e| {
                error!("Failed to resolve client ownership: {e}");
                Box::new(Self::a2a_error(
                    SERVER_ERROR_START,
                    "Failed to resolve client ownership",
                    request_id.cloned(),
                ))
            })?;
        owned.into_iter().next().ok_or_else(|| {
            Box::new(Self::a2a_error(
                SERVER_ERROR_START,
                "Failed to resolve client ownership",
                request_id.cloned(),
            ))
        })
    }

    /// Resolve the target task for an incoming message (continuation or
    /// creation), persist the user message into its history, and return the
    /// state shared by the blocking/background/streaming paths.
    async fn prepare_send(
        resources: &A2AResources,
        principal: &AuthPrincipal,
        mut message: Message,
        for_text_turn: bool,
        request_id: Option<&Value>,
    ) -> Result<PreparedSend, Box<A2AResponse>> {
        let repos = resources.ctx.repos();

        let (task_id, context_id, mut history) = if let Some(task_id) = message.task_id.clone() {
            // Continuation: the referenced task must exist (client-created
            // task ids are unsupported in A2A 1.0) and belong to the caller.
            let record = Self::load_owned_task(resources, &task_id, principal, request_id).await?;
            // A running task is its own: a second message would end it under
            // the turn still answering. A text turn continues by `contextId`.
            if record.status.is_terminal() || record.status == TaskStatus::Working {
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

            // A coaching reply is the athlete's, and stays out of reach of any
            // client they registered, whose client-credentials token acts for
            // no athlete.
            let client_id = if for_text_turn {
                user_task_key(principal.user_id)
            } else {
                Self::resolve_task_client(principal, resources, request_id).await?
            };

            let input = to_value(&message).map_err(|e| {
                error!("Failed to serialize A2A message: {e}");
                Box::new(Self::a2a_error(
                    INTERNAL_ERROR,
                    "Failed to serialize message",
                    request_id.cloned(),
                ))
            })?;

            let task_id = repos
                .a2a
                .create_task(&client_id, None, "message", &input, Some(&context_id))
                .await
                .map_err(|e| {
                    error!("Failed to persist task: {e}");
                    Box::new(Self::a2a_error(
                        SERVER_ERROR_START,
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
                INTERNAL_ERROR,
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
                    SERVER_ERROR_START,
                    "Failed to persist task history",
                    request_id.cloned(),
                ))
            })?;

        Ok(PreparedSend {
            task_id,
            context_id,
            user_message: message,
            history,
            text_turn: None,
        })
    }

    /// Execute a prepared message against its task: transition to `working`,
    /// run the tool intent, persist history/artifacts/status,
    /// publish stream events, close the stream, and deliver push
    /// notifications. Infallible by design — failures land on the task as
    /// the `failed` state.
    // Takes the principal, not a user id: it carries the caller's grant.
    async fn run_task_message(
        ctx: &Arc<dyn A2ACtx>,
        tool_runtime: &Arc<dyn ToolRuntime>,
        principal: AuthPrincipal,
        prepared: PreparedSend,
    ) {
        let PreparedSend {
            task_id,
            context_id,
            user_message,
            history,
            text_turn,
        } = prepared;

        Self::transition_status(ctx, &task_id, &context_id, TaskState::Working, None).await;

        let outcome = match (text_turn, Self::extract_tool_intent(&user_message)) {
            (Some(turn), _) => Self::run_text_turn(ctx, &task_id, turn).await,
            (None, Some((tool_name, params))) => {
                // A live support session runs the call as its user
                // (carnet#1511), and its task is read only while it lives
                // (call 28).
                match Self::acting_for(ctx.as_ref(), &principal, &tool_name).await {
                    Err(refused) => SendOutcome::Failure(SendFailure::text(refused)),
                    Ok(acting) => {
                        let support = acting.support.as_ref().map(|live| live.session.id.clone());
                        let run = Self::execute_registered_tool(
                            ctx,
                            tool_runtime,
                            &principal,
                            acting,
                            &tool_name,
                            params,
                        );
                        Self::recording_members(ctx, &task_id, support, run)
                            .await
                            .map_or_else(SendOutcome::Failure, |(result, labels)| {
                                SendOutcome::Tool(tool_name, result, labels)
                            })
                    }
                }
            }
            // A message addressing no registered tool has nothing to run:
            // the task fails with the shape the surface accepts.
            (None, None) => SendOutcome::Failure(SendFailure::text(NO_TOOL_INTENT_MESSAGE)),
        };

        match outcome {
            // The cancel closed the stream and notified the webhooks.
            SendOutcome::Canceled => return,
            SendOutcome::Coach(reply) => {
                let artifact = Artifact {
                    artifact_id: Uuid::new_v4().to_string(),
                    parts: vec![Part::text(reply.clone())], // Safe: String ownership for part
                    name: Some(text_turn::REPLY_ARTIFACT_NAME.to_owned()),
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
                    reply,
                    None,
                )
                .await;
            }
            SendOutcome::Failure(failure) => {
                Self::finalize_failed(ctx, &task_id, &context_id, history, failure).await;
            }
            SendOutcome::Tool(tool_name, result, labels) => {
                // The trust label rides on the artifact's metadata, for the
                // caller's host, as it rides on an MCP result's `_meta`.
                let metadata = labels
                    .map(|labels| Map::from_iter([(TRUST_LABELS_META_KEY.to_owned(), labels)]));
                let artifact = Artifact {
                    artifact_id: Uuid::new_v4().to_string(),
                    parts: vec![Part::data(result.clone())], // Safe: JSON value ownership for part
                    name: Some(format!("{tool_name} result")),
                    description: None,
                    extensions: Vec::new(),
                    metadata,
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
        }

        // Terminal state reached: end all subscriptions and notify webhooks.
        TASK_EVENTS.close(&task_id);
        Self::notify_webhooks(ctx, &task_id).await;
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

    /// An admin-only call refused before it reaches the executor, which
    /// writes the audit row of every other one, is written `refused` here: every
    /// admin-only call over A2A is on the trail (EFVP 9.4d). An unknown tool is
    /// no admin-only one, so it writes nothing.
    /// `credential` is the one the call came through, `None` when the
    /// credential itself was refused.
    async fn record_refused(
        tool_runtime: &Arc<dyn ToolRuntime>,
        user_id: Uuid,
        tool_name: &str,
        credential: Option<&CallerCredential>,
    ) {
        record_refused_admin_call(
            tool_runtime.as_ref(),
            &RefusedAdminCall {
                tool_name,
                user_id,
                tenant_id: None,
                transport: Transport::A2a,
                protocol: "a2a",
                credential,
            },
        )
        .await;
    }

    /// Record the admin-only call a message names, `tool`, refused before
    /// its task ran (a continuation of a finished or running task, a task
    /// that could not be stored, a push config refused): every admin-only
    /// call over A2A is on the trail (EFVP 9.4d). A message naming no tool
    /// writes nothing.
    async fn record_refused_send(
        resources: &A2AResources,
        principal: &AuthPrincipal,
        tool: Option<&str>,
    ) {
        if let Some(tool) = tool {
            Self::record_refused(
                &resources.tool_runtime,
                principal.user_id,
                tool,
                Some(&principal.credential),
            )
            .await;
        }
    }

    /// Execute a registered tool against the authenticated user's tenant.
    /// Returns the tool output and its trust label on success, or the failure
    /// (surfaced on the task as the `failed` status message).
    async fn execute_registered_tool(
        ctx: &Arc<dyn A2ACtx>,
        tool_runtime: &Arc<dyn ToolRuntime>,
        principal: &AuthPrincipal,
        acting: support::Acting,
        tool_name: &str,
        tool_params: Value,
    ) -> Result<(Value, Option<Value>), SendFailure> {
        let user_id = acting.user_id;
        // Resolve tenant context — required for all A2A tool execution.
        let tenant = acting.tenant;
        let tenant_context =
            match extract_tenant_context_internal(ctx.repos(), user_id, tenant).await {
                Ok(Some(context)) => context,
                Ok(None) => {
                    Self::record_refused(
                        tool_runtime,
                        user_id,
                        tool_name,
                        Some(&principal.credential),
                    )
                    .await;
                    return Err(SendFailure::text("User does not belong to any tenant"));
                }
                Err(e) => {
                    log_tenant_failure(user_id, &e);
                    Self::record_refused(
                        tool_runtime,
                        user_id,
                        tool_name,
                        Some(&principal.credential),
                    )
                    .await;
                    return Err(SendFailure::text("Failed to resolve tenant context"));
                }
            };

        if !tool_runtime.tool_registry().contains(tool_name) {
            return Err(SendFailure::text(format!("Unknown tool: {tool_name}")));
        }

        // The unified executor: Guardian chokepoint and admin/tenant context as on MCP and
        // chat, under the client's own grant (unbound, it refuses). A2A is outside Dravr's
        // own surfaces (carnet#724), and the agent's reply goes unchecked (carnet#852).
        let request = UniversalRequest {
            tool_name: tool_name.to_owned(),
            parameters: tool_params,
            user_id: user_id.to_string(),
            protocol: "a2a".to_owned(),
            tenant_id: Some(tenant_context.tenant_id.to_string()),
        };
        // Built inside the session's scope: the executor reads the session
        // when it is built, and carries it into the tool body.
        let credential = principal.credential.clone();
        let session = acting
            .support
            .as_ref()
            .map(support_session::LiveSupport::grant);
        let scopes = acting.scopes;
        let run = async {
            UniversalToolExecutor::new(tool_runtime.clone()) // Safe: Arc clone
                .with_scopes(scopes)
                .with_ceiling(principal.ceiling.clone())
                .with_transport(Transport::A2a)
                .with_caller_writing_reply()
                .execute_tool(request)
                .await
        };
        let executed = CALLER_CREDENTIAL
            .scope(Some(credential), SUPPORT_SESSION.scope(session, run))
            .await;

        match executed {
            Ok(response) if response.success => {
                let labels = trust_labels::labels_of(&response).cloned();
                let result = response
                    .result
                    .unwrap_or_else(|| Value::Object(Map::default()));
                Ok((result, labels))
            }
            Ok(response) => {
                let data = guardian_refusal_data(&response);
                Err(SendFailure {
                    reason: response
                        .error
                        .unwrap_or_else(|| "Tool execution failed".into()),
                    data,
                })
            }
            Err(e) => Err(SendFailure::text(e.to_string())),
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
        let principal = Self::authenticate_request(&request, resources).await?;

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
        let mut message = params.message;

        let text_turn =
            Self::admit_text_turn(resources, &principal, &mut message, request.id.as_ref()).await?;

        let tool = Self::extract_tool_intent(&message).map(|(tool, _)| tool);
        let mut prepared = match Self::prepare_send(
            resources,
            &principal,
            message,
            text_turn.is_some(),
            request.id.as_ref(),
        )
        .await
        {
            Ok(prepared) => prepared,
            Err(err) => {
                Self::record_refused_send(resources, &principal, tool.as_deref()).await;
                return Err(err);
            }
        };
        prepared.text_turn = text_turn;

        if let Some(config_input) = configuration.push_notification_config {
            if let Err(err) = Self::store_push_config(
                resources,
                &prepared.task_id,
                config_input,
                request.id.as_ref(),
            )
            .await
            {
                Self::record_refused_send(resources, &principal, tool.as_deref()).await;
                return Err(err);
            }
        }

        // Subscribe before spawning so no event can be missed.
        let receiver = TASK_EVENTS.subscribe(&prepared.task_id);

        let snapshot = match Self::load_owned_task(
            resources,
            &prepared.task_id,
            &principal,
            request.id.as_ref(),
        )
        .await
        {
            Ok(record) => Self::wire_task(&record, configuration.history_length, true),
            Err(err) => return Err(err),
        };

        let ctx = resources.ctx.clone(); // Safe: Arc clone for spawned executor
        let tool_runtime = resources.tool_runtime.clone(); // Safe: Arc clone for spawned executor
                                                           // Detached by design: completion is signalled by stream closure. Spawned
                                                           // under the server's in-flight tracker so a drain waits for it once the
                                                           // subscriber has gone; the reaper fails whatever the drain still loses.
        let spawner = Arc::clone(&resources.ctx);
        spawner.spawn_detached_task(Box::pin(async move {
            Self::run_task_message(&ctx, &tool_runtime, principal, prepared).await;
        }));

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
        let principal = Self::authenticate_request(&request, resources).await?;

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
            Self::load_owned_task(resources, &params.id, &principal, request.id.as_ref()).await?;

        if record.status.is_terminal() {
            return Err(Box::new(Self::spec_error(
                A2ASpecError::UnsupportedOperation,
                request.id,
            )));
        }

        let receiver = TASK_EVENTS.subscribe(&record.id);
        let snapshot = Self::readable_wire_task(
            resources,
            principal.user_id,
            &record,
            params.history_length,
            true,
        )
        .await;
        Ok((snapshot, receiver))
    }
}

impl Default for A2AServer {
    fn default() -> Self {
        Self::new()
    }
}
