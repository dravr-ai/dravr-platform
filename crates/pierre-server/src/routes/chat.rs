// ABOUTME: Chat route handlers for AI conversation management
// ABOUTME: Provides REST endpoints for creating, listing, and messaging in chat conversations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Chat routes for AI conversations
//!
//! This module handles chat conversation management including creating conversations,
//! sending messages, and streaming AI responses. All handlers require JWT authentication.

use super::chat_tool_loop::{self, ToolLoopParams};
use crate::agui::{AgUiEventFilter, BroadcastSink, RunOwner, RunScope};
use crate::models::TenantId;
use crate::services::chat_pipeline;
use crate::services::chat_pipeline::stages::persistence::{
    persist_assistant_response, persist_user_message,
};
use crate::{
    errors::AppError,
    llm::{ChatMessage, ChatProvider, Tool},
    mcp::resources::ServerResources,
    middleware::extract_auth_from_headers,
    protocols::universal::UniversalExecutor,
    services::{
        chat_orchestration, chat_verdicts,
        usage_counter::{LimitCheckResult, UsageCounterService},
    },
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Json, Router,
};
use pierre_auth::auth::AuthResult;
use pierre_core::models::usage::InsertLlmUsage;
use pierre_core::models::{AddMessageParams, ConversationTurnId, TURN_SUMMARY_CALL_TYPE};
use pierre_core::tokens::estimate_chat_tokens;
use pierre_database::database::ConversationRecord;
#[cfg(feature = "client-notifications")]
use pierre_notifications::triggers as notification_triggers;
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Instant};
use tracing::{debug, warn};
use uuid::Uuid;
// ============================================================================
// Constants
// ============================================================================

/// Default maximum number of tool call iterations before forcing a text response
const DEFAULT_MAX_TOOL_ITERATIONS: usize = 10;

/// Prefix used to detect insight generation requests from the frontend.
/// Must match the `INSIGHT_PROMPT_PREFIX` constant in `@pierre/chat-utils`.
const INSIGHT_PROMPT_PREFIX: &str = "Create a shareable insight from this analysis";

// ============================================================================
// Helper Functions
// ============================================================================

/// JSON response structure for insight generation
#[derive(Debug, Deserialize)]
struct InsightGenerationResponse {
    content: String,
}

/// Parse JSON response from insight generation prompt
///
/// The insight generation prompt returns JSON: `{"content": "..."}`
/// This extracts the content field, falling back to raw content if parsing fails.
fn parse_insight_json_response(raw_content: &str) -> String {
    // Try to parse as JSON
    if let Ok(response) = serde_json::from_str::<InsightGenerationResponse>(raw_content) {
        return response.content;
    }

    // Sometimes LLMs wrap JSON in markdown code blocks, try to extract
    let trimmed = raw_content.trim();
    if let Some(json_start) = trimmed.find('{') {
        if let Some(json_end) = trimmed.rfind('}') {
            let json_str = &trimmed[json_start..=json_end];
            if let Ok(response) = serde_json::from_str::<InsightGenerationResponse>(json_str) {
                return response.content;
            }
        }
    }

    // Fallback: return raw content with warning (avoid logging raw content which may contain user data)
    warn!(
        "Failed to parse insight generation JSON response, using raw content ({} bytes)",
        raw_content.len()
    );
    raw_content.to_owned()
}

// ============================================================================
// Internal Types
// ============================================================================

/// Parameters for recording the terminal turn-summary row after a
/// chat/insight turn. Per-LLM-call token counts are written separately
/// by the tool loop's [`crate::services::tool_execution::LlmCallRecorder`];
/// this struct only carries the turn-level aggregates.
struct RecordLlmUsageParams<'a> {
    /// Tenant this usage belongs to
    tenant_id: TenantId,
    /// User who initiated the request
    user_id: &'a str,
    /// Conversation this message belongs to
    conversation_id: &'a str,
    /// Conversation-turn identifier threaded from the inbound request
    turn_id: ConversationTurnId,
    /// LLM provider used for this completion
    provider: &'a ChatProvider,
    /// Model identifier used for this completion
    model: &'a str,
    /// Number of tool calls executed across the turn
    tool_calls_count: u32,
    /// MCP tool names invoked during the turn (duplicates preserved in call order)
    tools_called: &'a [String],
    /// Total wall-clock time for the LLM interaction in milliseconds
    execution_time_ms: u64,
}

// ============================================================================
// Request/Response Types
// ============================================================================

/// Request to create a new conversation
#[derive(Debug, Deserialize)]
pub struct CreateConversationRequest {
    /// Conversation title
    pub title: String,
    /// LLM model to use (optional, defaults to provider's default model)
    #[serde(default)]
    pub model: Option<String>,
    /// Coach ID to attach to this conversation (optional). The coach's
    /// system prompt is resolved at runtime from the `coaches` table.
    #[serde(default)]
    pub coach_id: Option<String>,
}

/// Response for conversation creation
#[derive(Debug, Serialize, Deserialize)]
pub struct ConversationResponse {
    /// Conversation ID
    pub id: String,
    /// Conversation title
    pub title: String,
    /// Model used
    pub model: String,
    /// Coach attached to this conversation, if any
    pub coach_id: Option<String>,
    /// Total tokens used
    pub total_tokens: i64,
    /// Creation timestamp
    pub created_at: String,
    /// Last update timestamp
    pub updated_at: String,
}

/// Response for listing conversations
#[derive(Debug, Serialize, Deserialize)]
pub struct ConversationListResponse {
    /// List of conversations
    pub conversations: Vec<ConversationSummaryResponse>,
    /// Total count
    pub total: usize,
}

/// Summary of a conversation for listing
#[derive(Debug, Serialize, Deserialize)]
pub struct ConversationSummaryResponse {
    /// Conversation ID
    pub id: String,
    /// Conversation title
    pub title: String,
    /// Model used
    pub model: String,
    /// Message count
    pub message_count: i64,
    /// Total tokens used
    pub total_tokens: i64,
    /// Creation timestamp
    pub created_at: String,
    /// Last update timestamp
    pub updated_at: String,
}

/// Request to update a conversation title
#[derive(Debug, Deserialize)]
pub struct UpdateConversationRequest {
    /// New title
    pub title: String,
}

/// Bundles the AG-UI sink and registry scope for a single chat turn.
///
/// The handler holds one of these across [`chat_pipeline::run`] so
/// events flow into the registry while the pipeline executes; the
/// [`crate::agui::RunScope`] inside `scope` auto-unregisters the run
/// when the wiring drops at handler exit.
struct AgUiWiring {
    scope: RunScope,
    sink: BroadcastSink,
    thread_id: String,
}

impl AgUiWiring {
    fn run_id(&self) -> &str {
        self.scope.run_id()
    }

    fn run(&self) -> chat_pipeline::AgUiRun<'_> {
        chat_pipeline::AgUiRun {
            run_id: self.scope.run_id().to_owned(),
            thread_id: Some(self.thread_id.clone()),
            sink: &self.sink,
        }
    }
}

/// Request to send a message
#[derive(Debug, Deserialize)]
pub struct SendMessageRequest {
    /// Message content
    pub content: String,
    /// Whether to stream the response
    #[serde(default)]
    pub stream: bool,
    /// Optional caller-supplied AG-UI `run_id` (UUID).
    ///
    /// When present the server registers the run under this id and
    /// emits AG-UI events the client can consume in parallel via
    /// `GET /api/agui/runs/{run_id}/stream`.
    ///
    /// Use a fresh UUID per turn. Clients that do not care about
    /// progress feedback should omit the field — the pipeline runs
    /// without AG-UI overhead in that case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agui_run_id: Option<String>,
}

/// Response for a message
#[derive(Debug, Serialize, Deserialize)]
pub struct MessageResponse {
    /// Message ID
    pub id: String,
    /// Role (user/assistant/system)
    pub role: String,
    /// Message content
    pub content: String,
    /// Token count
    pub token_count: Option<i64>,
    /// Creation timestamp
    pub created_at: String,
}

/// Response with chat completion (non-streaming)
#[derive(Debug, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    /// User message
    pub user_message: MessageResponse,
    /// Assistant response
    pub assistant_message: MessageResponse,
    /// Conversation updated timestamp
    pub conversation_updated_at: String,
    /// LLM model used for the response
    pub model: String,
    /// Total execution time in milliseconds (including tool calls)
    pub execution_time_ms: u64,
    /// Activity list from `get_activities` tool, kept separate from message content
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_list: Option<String>,
    /// AG-UI `run_id` echoed back when the request supplied one. The
    /// caller uses it to correlate this turn with its parallel
    /// `/api/agui/runs/{run_id}/stream` subscription.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agui_run_id: Option<String>,
}

/// Response for messages list
#[derive(Debug, Serialize, Deserialize)]
pub struct MessagesListResponse {
    /// List of messages
    pub messages: Vec<MessageResponse>,
}

/// Query parameters for listing conversations
#[derive(Debug, Deserialize, Default)]
pub struct ListConversationsQuery {
    /// Maximum number of conversations to return
    #[serde(default = "default_limit")]
    pub limit: i64,
    /// Offset for pagination
    #[serde(default)]
    pub offset: i64,
}

const fn default_limit() -> i64 {
    20
}

// ============================================================================
// Chat Routes
// ============================================================================

/// Chat routes handler
pub struct ChatRoutes;

impl ChatRoutes {
    /// Create all chat routes
    pub fn routes(resources: Arc<ServerResources>) -> Router {
        Router::new()
            // Conversation management
            .route("/api/chat/conversations", post(Self::create_conversation))
            .route("/api/chat/conversations", get(Self::list_conversations))
            .route(
                "/api/chat/conversations/{conversation_id}",
                get(Self::get_conversation),
            )
            .route(
                "/api/chat/conversations/{conversation_id}",
                put(Self::update_conversation),
            )
            .route(
                "/api/chat/conversations/{conversation_id}",
                delete(Self::delete_conversation),
            )
            // Messages
            .route(
                "/api/chat/conversations/{conversation_id}/messages",
                get(Self::get_messages),
            )
            // POST messages with MCP tool support (non-streaming)
            .route(
                "/api/chat/conversations/{conversation_id}/messages",
                post(Self::send_message),
            )
            // Tier 5.5 claim verdicts attached to messages in this conversation
            .route(
                "/api/chat/conversations/{conversation_id}/verdicts",
                get(chat_verdicts::get_verdicts_handler),
            )
            .with_state(resources)
    }

    /// Extract and authenticate user from authorization header or cookie
    async fn authenticate(
        headers: &HeaderMap,
        resources: &Arc<ServerResources>,
    ) -> Result<AuthResult, AppError> {
        extract_auth_from_headers(headers, resources).await
    }

    /// Get user's `tenant_id` (defaults to `user_id` if no tenant)
    async fn get_tenant_id(
        user_id: Uuid,
        resources: &Arc<ServerResources>,
    ) -> Result<TenantId, AppError> {
        let tenants = resources.repos.tenants.list_for_user(user_id).await?;
        Ok(tenants
            .first()
            .map_or_else(|| TenantId::from(user_id), |t| t.id))
    }

    /// Get LLM provider based on `PIERRE_LLM_PROVIDER` environment variable
    async fn get_llm_provider() -> Result<ChatProvider, AppError> {
        super::create_chat_provider().await
    }

    /// Wire up AG-UI progress feedback for a single turn, when the
    /// caller asked for it via `agui_run_id`.
    ///
    /// The returned [`AgUiWiring`] owns:
    /// - a [`crate::agui::RunScope`] that auto-unregisters the run on drop
    ///   (closing any live SSE subscribers cleanly)
    /// - a [`BroadcastSink`] that emits to that registry slot
    ///
    /// The handler keeps the wiring alive across
    /// [`chat_pipeline::run`] so AG-UI events flow during the turn,
    /// then drops it after the response body is built.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::invalid_input`] when the caller-supplied
    /// `run_id` is not a UUID — short / guessable ids open a
    /// brute-force surface even with the per-run owner check.
    fn setup_agui(
        resources: &Arc<ServerResources>,
        requested_run_id: Option<&str>,
        user_id: Uuid,
        tenant_id: TenantId,
        conversation_id: &str,
    ) -> Result<Option<AgUiWiring>, AppError> {
        let Some(raw) = requested_run_id else {
            return Ok(None);
        };
        let parsed = Uuid::parse_str(raw).map_err(|_| {
            AppError::invalid_input(
                "agui_run_id must be a UUID string; use Uuid::new_v4() per turn",
            )
        })?;
        let run_id = parsed.to_string();
        let owner = RunOwner::new(user_id, tenant_id);
        let scope = resources.agui_registry.register_scoped(&run_id, owner);
        let sink = BroadcastSink::new(
            (*resources.agui_registry).clone(),
            AgUiEventFilter::default(),
        );
        Ok(Some(AgUiWiring {
            scope,
            sink,
            thread_id: conversation_id.to_owned(),
        }))
    }

    /// Build LLM tool definitions for chat-mode function calling.
    pub(crate) fn build_mcp_tools() -> Tool {
        use crate::services::tool_execution::build_mcp_tools as service_build;
        service_build()
    }

    // ========================================================================
    // Conversation Handlers
    // ========================================================================

    /// Create a new conversation
    async fn create_conversation(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Json(request): Json<CreateConversationRequest>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(auth.user_id, &resources).await?;
        let user_id_str = auth.user_id.to_string();

        // Enforce max_active_conversations limit from admin config
        if let Some(ref admin_config) = resources.admin_config {
            let max_conversations = admin_config
                .get_value(
                    "usage_quotas.max_active_conversations",
                    Some(&tenant_id.to_string()),
                )
                .await
                .ok()
                .flatten()
                .and_then(|v| v.as_i64())
                .unwrap_or(2);

            let current_count = resources
                .repos
                .chat
                .count_conversations(&user_id_str, tenant_id)
                .await?;
            if current_count >= max_conversations {
                return Err(AppError::quota_exceeded(
                    "max_active_conversations",
                    current_count,
                    max_conversations,
                    "",
                ));
            }
        }

        let result = chat_orchestration::create_conversation(
            resources.repos.chat.as_ref(),
            &user_id_str,
            tenant_id,
            &request.title,
            request.model.as_deref(),
            request.coach_id.as_deref(),
        )
        .await?;

        let conv = result.conversation;
        let response = ConversationResponse {
            id: conv.id,
            title: conv.title,
            model: conv.model,
            coach_id: conv.coach_id,
            total_tokens: conv.total_tokens,
            created_at: conv.created_at,
            updated_at: conv.updated_at,
        };

        Ok((StatusCode::CREATED, Json(response)).into_response())
    }

    /// List user's conversations
    async fn list_conversations(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Query(query): Query<ListConversationsQuery>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(auth.user_id, &resources).await?;

        let conversations = resources
            .repos
            .chat
            .list_conversations(
                &auth.user_id.to_string(),
                tenant_id,
                query.limit,
                query.offset,
            )
            .await?;

        let total = conversations.len();
        let response = ConversationListResponse {
            conversations: conversations
                .into_iter()
                .map(|c| ConversationSummaryResponse {
                    id: c.id,
                    title: c.title,
                    model: c.model,
                    message_count: c.message_count,
                    total_tokens: c.total_tokens,
                    created_at: c.created_at,
                    updated_at: c.updated_at,
                })
                .collect(),
            total,
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Get a specific conversation
    async fn get_conversation(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(conversation_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(auth.user_id, &resources).await?;

        let conv = resources
            .repos
            .chat
            .get_conversation(&conversation_id, &auth.user_id.to_string(), tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found("Conversation not found"))?;

        let response = ConversationResponse {
            id: conv.id,
            title: conv.title,
            model: conv.model,
            coach_id: conv.coach_id,
            total_tokens: conv.total_tokens,
            created_at: conv.created_at,
            updated_at: conv.updated_at,
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Update a conversation title
    async fn update_conversation(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(conversation_id): Path<String>,
        Json(request): Json<UpdateConversationRequest>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(auth.user_id, &resources).await?;

        let updated = resources
            .repos
            .chat
            .update_conversation_title(
                &conversation_id,
                &auth.user_id.to_string(),
                tenant_id,
                &request.title,
            )
            .await?;

        if !updated {
            return Err(AppError::not_found("Conversation not found"));
        }

        // Fetch and return the updated conversation (proper REST response)
        let conv = resources
            .repos
            .chat
            .get_conversation(&conversation_id, &auth.user_id.to_string(), tenant_id)
            .await?
            .ok_or_else(|| AppError::internal("Conversation not found after update"))?;

        let response = ConversationResponse {
            id: conv.id,
            title: conv.title,
            model: conv.model,
            coach_id: conv.coach_id,
            total_tokens: conv.total_tokens,
            created_at: conv.created_at,
            updated_at: conv.updated_at,
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Delete a conversation
    async fn delete_conversation(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(conversation_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(auth.user_id, &resources).await?;

        let deleted = resources
            .repos
            .chat
            .delete_conversation(&conversation_id, &auth.user_id.to_string(), tenant_id)
            .await?;

        if !deleted {
            return Err(AppError::not_found("Conversation not found"));
        }

        Ok((StatusCode::NO_CONTENT, ()).into_response())
    }

    // ========================================================================
    // Message Handlers
    // ========================================================================

    /// Get messages for a conversation
    async fn get_messages(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(conversation_id): Path<String>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(auth.user_id, &resources).await?;

        // Verify user owns this conversation
        resources
            .repos
            .chat
            .get_conversation(&conversation_id, &auth.user_id.to_string(), tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found("Conversation not found"))?;

        let messages = resources
            .repos
            .chat
            .get_messages(&conversation_id, &auth.user_id.to_string())
            .await?;

        let messages_list: Vec<MessageResponse> = messages
            .into_iter()
            .map(|m| MessageResponse {
                id: m.id,
                role: m.role,
                content: m.content,
                token_count: m.token_count,
                created_at: m.created_at,
            })
            .collect();

        let response = MessagesListResponse {
            messages: messages_list,
        };

        Ok((StatusCode::OK, Json(response)).into_response())
    }

    /// Send a message and get a response (non-streaming) with MCP tool execution.
    ///
    /// Insight-generation requests (prompts starting with
    /// `INSIGHT_PROMPT_PREFIX`) take a dedicated inline path — no coach, no
    /// tools, no memory — because they run on the insight-generation
    /// system prompt and expect JSON output. All other turns go through
    /// [`crate::services::chat_pipeline::run`].
    async fn send_message(
        State(resources): State<Arc<ServerResources>>,
        headers: HeaderMap,
        Path(conversation_id): Path<String>,
        Json(request): Json<SendMessageRequest>,
    ) -> Result<Response, AppError> {
        let auth = Self::authenticate(&headers, &resources).await?;
        let tenant_id = Self::get_tenant_id(auth.user_id, &resources).await?;
        let user_id_str = auth.user_id.to_string();
        let tenant_id_str = tenant_id.to_string();

        // Pre-chat quota check: verify message and token quotas before LLM dispatch.
        let usage_warning = Self::check_pre_chat_quotas(
            &resources,
            &tenant_id_str,
            &user_id_str,
            auth.user_id,
            tenant_id,
        )
        .await?;

        // Insight-generation requests bypass the unified pipeline (different
        // prompt, no coach, no tools, JSON response shape).
        if request.content.starts_with(INSIGHT_PROMPT_PREFIX) {
            // Insights are a one-shot JSON response — the unified-pipeline
            // AG-UI lifecycle (`RUN_STARTED` → steps → `RUN_FINISHED`) does
            // not fit. Rather than silently swallowing `agui_run_id` the
            // caller passed, reject it loudly so clients surface a clear
            // 400 instead of opening an SSE subscription that would never
            // receive any events.
            if request.agui_run_id.is_some() {
                return Err(AppError::invalid_input(
                    "agui_run_id is not supported on insight-generation requests; \
                     insights return a single JSON payload and emit no AG-UI events",
                ));
            }
            return Self::send_insight_message(
                resources,
                conversation_id,
                user_id_str,
                tenant_id,
                tenant_id_str,
                request,
                usage_warning,
            )
            .await;
        }

        let turn_input = chat_pipeline::TurnInput {
            conversation_id: conversation_id.clone(),
            user_id: user_id_str.clone(),
            conversation_tenant_id: tenant_id,
            tool_tenant_id: tenant_id,
            content: request.content.clone(),
            turn_id: ConversationTurnId::new(),
            // Web chat resolves the locale from the authenticated user's
            // profile; pipeline stages fall back to `DEFAULT_LOCALE` when
            // lookup fails so an empty locale never becomes an empty string.
            locale: resources
                .repos
                .users
                .get_global(auth.user_id)
                .await
                .ok()
                .flatten()
                .map(|u| u.locale),
        };
        let profile = chat_pipeline::ChannelProfile::web_chat();

        // Set up AG-UI progress feedback for callers that asked for it.
        // The scope guard auto-unregisters when this handler returns,
        // so a leak from an early `?` propagation is impossible.
        let agui_wiring = Self::setup_agui(
            &resources,
            request.agui_run_id.as_deref(),
            auth.user_id,
            tenant_id,
            &conversation_id,
        )?;

        let start_time = Instant::now();
        let hooks = chat_pipeline::PipelineHooks {
            agui: agui_wiring.as_ref().map(|w| w.run()),
            ..chat_pipeline::PipelineHooks::none()
        };
        let dispatch = chat_pipeline::run(&resources, turn_input, &profile, &hooks).await?;

        // Safe cast: execution time will never exceed u64::MAX milliseconds (~584 million years)
        #[allow(clippy::cast_possible_truncation)]
        let execution_time_ms = start_time.elapsed().as_millis() as u64;

        // Extract tokens for usage recording — use real values from the LLM
        // when available, fall back to character-based estimation otherwise.
        let (prompt_tokens, completion_tokens) =
            Self::tokens_from_dispatch(&dispatch, &request.content);

        // Record LLM usage for cost tracking and quota enforcement.
        let provider = Self::get_llm_provider().await?;
        Self::record_llm_usage(
            &resources,
            &RecordLlmUsageParams {
                tenant_id,
                user_id: &user_id_str,
                conversation_id: &conversation_id,
                turn_id: dispatch.turn_id,
                provider: &provider,
                model: &dispatch.model,
                tool_calls_count: dispatch.tool_calls_count,
                tools_called: &dispatch.tools_called,
                execution_time_ms,
            },
        )
        .await;

        let total_tokens_used =
            i64::from(prompt_tokens.unwrap_or(0)) + i64::from(completion_tokens.unwrap_or(0));
        Self::increment_usage_counters(
            &resources,
            &tenant_id_str,
            &user_id_str,
            total_tokens_used,
            dispatch.tool_calls_count,
        )
        .await;

        let response = ChatCompletionResponse {
            user_message: MessageResponse {
                id: dispatch.user_message.id.clone(),
                role: dispatch.user_message.role.clone(),
                content: dispatch.user_message.content.clone(),
                token_count: dispatch.user_message.token_count,
                created_at: dispatch.user_message.created_at,
            },
            assistant_message: MessageResponse {
                id: dispatch.assistant_message.id.clone(),
                role: dispatch.assistant_message.role.clone(),
                content: dispatch.assistant_message.content.clone(),
                token_count: dispatch.assistant_message.token_count,
                created_at: dispatch.assistant_message.created_at,
            },
            conversation_updated_at: dispatch.conversation.updated_at.clone(),
            model: dispatch.model.clone(),
            execution_time_ms,
            activity_list: dispatch.activity_list.clone(),
            agui_run_id: agui_wiring.as_ref().map(|w| w.run_id().to_owned()),
        };

        // Notify user when a coach conversation produces a response.
        #[cfg(feature = "client-notifications")]
        Self::notify_coach_response(
            &resources,
            &dispatch.conversation,
            auth.user_id,
            tenant_id,
            &conversation_id,
        );

        let mut http_response = (StatusCode::OK, Json(response)).into_response();
        Self::apply_usage_warning_headers(&mut http_response, usage_warning);
        Ok(http_response)
    }

    /// Dispatch an insight-generation request.
    ///
    /// Insight requests do not go through the unified chat pipeline — they
    /// run on the dedicated insight-generation prompt, skip coach context
    /// and tools entirely, and expect the LLM to emit structured JSON that
    /// is parsed out of the raw reply.
    #[allow(clippy::too_many_arguments)]
    async fn send_insight_message(
        resources: Arc<ServerResources>,
        conversation_id: String,
        user_id_str: String,
        tenant_id: TenantId,
        tenant_id_str: String,
        request: SendMessageRequest,
        usage_warning: Option<(&'static str, i64, i64, String)>,
    ) -> Result<Response, AppError> {
        // Verify ownership and persist user message.
        let msg_result = persist_user_message(
            resources.repos.chat.as_ref(),
            &conversation_id,
            &user_id_str,
            tenant_id,
            &request.content,
        )
        .await?;
        let conv = msg_result.conversation;
        let user_msg = msg_result.message;

        let insight_prompt = resources.insight_generation_prompt();
        let analysis_content = request
            .content
            .strip_prefix(INSIGHT_PROMPT_PREFIX)
            .unwrap_or(&request.content)
            .trim_start_matches(':')
            .trim();
        let mut llm_messages = vec![
            ChatMessage::system(insight_prompt),
            ChatMessage::user(analysis_content),
        ];

        let tools = Self::build_mcp_tools();
        let provider = Self::get_llm_provider().await?;
        let executor = Arc::new(UniversalExecutor::new(resources.clone()));

        let turn_id = ConversationTurnId::new();
        let start_time = Instant::now();
        // Per-LLM-call recorder so every insight-tool-loop iteration
        // writes its own `llm_usage` row under this turn id.
        let call_recorder: Option<Arc<dyn chat_tool_loop::LlmCallRecorder>> =
            Some(Arc::new(chat_pipeline::TurnCallRecorder::new(
                Arc::clone(&resources.repos.llm_usage),
                tenant_id_str.clone(),
                user_id_str.clone(),
                Some(conversation_id.clone()),
                turn_id,
                "insight",
            )));
        let tool_params = ToolLoopParams {
            provider: &provider,
            executor: Arc::clone(&executor),
            tools: &tools,
            model: &conv.model,
            user_id: &user_id_str,
            tenant_id,
            max_iterations: DEFAULT_MAX_TOOL_ITERATIONS,
            call_recorder,
            temperature: None,
        };
        let result = chat_tool_loop::run_tool_loop(&tool_params, &mut llm_messages).await?;

        #[allow(clippy::cast_possible_truncation)]
        let execution_time_ms = start_time.elapsed().as_millis() as u64;

        let (prompt_tokens, token_count) = Self::extract_or_estimate_tokens(&result, &llm_messages);

        let final_content = Self::post_process_content(&result.content, true);

        let assistant_params = AddMessageParams {
            conversation_id: &conversation_id,
            user_id: &user_id_str,
            role: "assistant",
            content: &final_content,
            token_count,
            finish_reason: result.finish_reason.as_deref(),
            prompt_tokens,
            model: Some(&conv.model),
        };
        let (assistant_msg, updated_conv) =
            persist_assistant_response(resources.repos.chat.as_ref(), &assistant_params, tenant_id)
                .await?;

        Self::record_llm_usage(
            &resources,
            &RecordLlmUsageParams {
                tenant_id,
                user_id: &user_id_str,
                conversation_id: &conversation_id,
                turn_id,
                provider: &provider,
                model: &conv.model,
                tool_calls_count: result.tool_calls_count,
                tools_called: &result.tools_called,
                execution_time_ms,
            },
        )
        .await;

        let total_tokens_used =
            i64::from(prompt_tokens.unwrap_or(0)) + i64::from(token_count.unwrap_or(0));
        Self::increment_usage_counters(
            &resources,
            &tenant_id_str,
            &user_id_str,
            total_tokens_used,
            result.tool_calls_count,
        )
        .await;

        let response = ChatCompletionResponse {
            user_message: MessageResponse {
                id: user_msg.id,
                role: user_msg.role,
                content: user_msg.content,
                token_count: user_msg.token_count,
                created_at: user_msg.created_at,
            },
            assistant_message: MessageResponse {
                id: assistant_msg.id,
                role: assistant_msg.role,
                content: assistant_msg.content,
                token_count: assistant_msg.token_count,
                created_at: assistant_msg.created_at,
            },
            conversation_updated_at: updated_conv.updated_at,
            model: conv.model.clone(),
            execution_time_ms,
            activity_list: result.activity_list,
            // Insight requests bypass the unified pipeline and never
            // emit AG-UI events; the field is omitted from the JSON
            // body via `skip_serializing_if = "Option::is_none"`.
            agui_run_id: None,
        };

        let mut http_response = (StatusCode::OK, Json(response)).into_response();
        Self::apply_usage_warning_headers(&mut http_response, usage_warning);
        Ok(http_response)
    }

    /// Resolve prompt/completion token counts from a pipeline dispatch result.
    ///
    /// Prefers real provider-reported counts from [`TokenUsage`]. When the
    /// provider does not report usage (CLI-based providers such as
    /// Copilot headless), falls back to character-based estimation on the
    /// user's input for the prompt side and on the assistant reply for
    /// the completion side — matching what the inline dispatch used to do.
    fn tokens_from_dispatch(
        dispatch: &chat_pipeline::DispatchResult,
        user_content: &str,
    ) -> (Option<u32>, Option<u32>) {
        dispatch.usage.as_ref().map_or_else(
            || {
                let (prompt_est, completion_est) =
                    estimate_chat_tokens(user_content, &dispatch.content);
                (Some(prompt_est), Some(completion_est))
            },
            |usage| (Some(usage.prompt_tokens), Some(usage.completion_tokens)),
        )
    }

    /// Fire-and-forget notification when a coach conversation produces a response.
    /// Only sends if the conversation has a `coach_id` (indicates coach persona).
    #[cfg(feature = "client-notifications")]
    fn notify_coach_response(
        resources: &Arc<ServerResources>,
        conv: &ConversationRecord,
        user_id: Uuid,
        tenant_id: TenantId,
        conversation_id: &str,
    ) {
        if conv.coach_id.is_some() {
            if let Some(service) = &resources.notification_service {
                let coach_title = conv.title.clone();
                notification_triggers::trigger_coach_message(
                    service,
                    user_id,
                    pierre_notifications::TenantId(tenant_id.0),
                    conversation_id,
                    &coach_title,
                );
            }
        }
    }

    /// Check pre-chat quotas and return optional usage warning info for response headers.
    ///
    /// Checks daily messages (with burst), weekly tokens (hard cap), and daily tokens (with burst).
    /// Returns `Err` with 429 if any hard limit is exceeded.
    /// Admin and owner roles bypass quota enforcement entirely.
    async fn check_pre_chat_quotas(
        resources: &Arc<ServerResources>,
        tenant_id: &str,
        user_id: &str,
        user_uuid: Uuid,
        tenant_uuid: TenantId,
    ) -> Result<Option<(&'static str, i64, i64, String)>, AppError> {
        let Some(ref admin_config) = resources.admin_config else {
            debug!("Admin config not available, skipping quota check");
            return Ok(None);
        };

        // Admin role bypasses quota enforcement for debugging and testing.
        // Owners (tenant creators) remain subject to quotas as cost control.
        if let Ok(Some(role)) = resources
            .repos
            .tenants
            .get_user_role(user_uuid, tenant_uuid)
            .await
        {
            if role == "admin" {
                debug!("Skipping quota check for admin user {user_id}");
                return Ok(None);
            }
        }

        let usage_svc =
            UsageCounterService::new(resources.repos.usage_counters.as_ref(), admin_config);

        // Check daily message quota (allows 1.5x burst)
        let daily_msg_check = usage_svc
            .check_limit(tenant_id, user_id, "daily_messages")
            .await?;
        if !daily_msg_check.allowed {
            return Err(AppError::quota_exceeded(
                "daily_messages",
                daily_msg_check.current,
                daily_msg_check.limit,
                &daily_msg_check.resets_at,
            ));
        }

        // Check weekly token budget (hard cap, no burst allowed)
        let weekly_token_check = usage_svc
            .check_limit(tenant_id, user_id, "weekly_tokens")
            .await?;
        if weekly_token_check.current >= weekly_token_check.limit {
            return Err(AppError::quota_exceeded(
                "weekly_tokens",
                weekly_token_check.current,
                weekly_token_check.limit,
                &weekly_token_check.resets_at,
            ));
        }

        // Check daily token budget (allows 1.5x burst)
        let daily_token_check = usage_svc
            .check_limit(tenant_id, user_id, "daily_tokens")
            .await?;
        if !daily_token_check.allowed {
            return Err(AppError::quota_exceeded(
                "daily_tokens",
                daily_token_check.current,
                daily_token_check.limit,
                &daily_token_check.resets_at,
            ));
        }

        // Track the most restrictive warning/burst state for response headers
        Ok(Self::select_usage_warning(
            &daily_msg_check,
            &daily_token_check,
            &weekly_token_check,
        ))
    }

    /// Select the most restrictive usage warning from daily and weekly checks
    ///
    /// Priority: burst zone > approaching warning. Within each tier, weekly caps
    /// take precedence over daily since they represent a harder boundary.
    fn select_usage_warning(
        daily_msg_check: &LimitCheckResult,
        daily_token_check: &LimitCheckResult,
        weekly_token_check: &LimitCheckResult,
    ) -> Option<(&'static str, i64, i64, String)> {
        let checks: &[&LimitCheckResult] =
            &[weekly_token_check, daily_token_check, daily_msg_check];

        // Burst zone takes highest priority (most restrictive)
        if let Some(check) = checks.iter().find(|c| c.burst_zone) {
            return Some(("burst", check.current, check.limit, check.resets_at.clone()));
        }

        // Warning threshold is next priority
        if let Some(check) = checks.iter().find(|c| c.warning) {
            return Some((
                "approaching",
                check.current,
                check.limit,
                check.resets_at.clone(),
            ));
        }

        None
    }

    /// Apply usage warning headers to an HTTP response
    fn apply_usage_warning_headers(
        response: &mut Response,
        warning: Option<(&str, i64, i64, String)>,
    ) {
        if let Some((level, current, limit, resets_at)) = warning {
            let headers = response.headers_mut();
            if let Ok(val) = HeaderValue::from_str(level) {
                headers.insert("X-Usage-Warning", val);
            }
            if let Ok(val) = HeaderValue::from_str(&current.to_string()) {
                headers.insert("X-Usage-Current", val);
            }
            if let Ok(val) = HeaderValue::from_str(&limit.to_string()) {
                headers.insert("X-Usage-Limit", val);
            }
            if let Ok(val) = HeaderValue::from_str(&resets_at) {
                headers.insert("X-Usage-Resets-At", val);
            }
        }
    }

    /// Post-process LLM content: parse insight JSON if applicable
    fn post_process_content(raw_content: &str, is_insight_request: bool) -> String {
        if is_insight_request {
            parse_insight_json_response(raw_content)
        } else {
            raw_content.to_owned()
        }
    }

    /// Increment usage counters after a successful LLM call
    ///
    /// Tracks daily/weekly messages, tokens, and tool calls. Failures are logged
    /// but do not block the chat response to avoid degrading user experience.
    async fn increment_usage_counters(
        resources: &Arc<ServerResources>,
        tenant_id: &str,
        user_id: &str,
        total_tokens: i64,
        tool_calls_count: u32,
    ) {
        let Some(ref admin_config) = resources.admin_config else {
            return;
        };

        let usage_svc =
            UsageCounterService::new(resources.repos.usage_counters.as_ref(), admin_config);

        // Build list of (counter_type, amount) pairs to increment
        let mut counters: Vec<(&str, i64)> = vec![("daily_messages", 1), ("weekly_messages", 1)];
        if total_tokens > 0 {
            counters.push(("daily_tokens", total_tokens));
            counters.push(("weekly_tokens", total_tokens));
        }
        if tool_calls_count > 0 {
            let tool_calls = i64::from(tool_calls_count);
            counters.push(("daily_tool_calls", tool_calls));
            counters.push(("weekly_tool_calls", tool_calls));
        }

        for (counter_type, amount) in counters {
            if let Err(e) = usage_svc
                .increment(tenant_id, user_id, counter_type, amount)
                .await
            {
                warn!("Failed to increment {counter_type} counter: {e}");
            }
        }
    }

    /// Extract real token counts from provider usage, or estimate from content length.
    ///
    /// CLI-based and headless providers (e.g., `copilot_headless`, Claude Code) return
    /// `usage: None`. In that case, we estimate tokens from the conversation text
    /// using a character-based heuristic (~4 chars per token).
    fn extract_or_estimate_tokens(
        result: &chat_tool_loop::ToolLoopResult,
        llm_messages: &[ChatMessage],
    ) -> (Option<u32>, Option<u32>) {
        result.usage.as_ref().map_or_else(
            || {
                let prompt_text: String = llm_messages
                    .iter()
                    .map(|m| m.content.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                let (est_prompt, est_completion) =
                    estimate_chat_tokens(&prompt_text, &result.content);
                debug!(
                    est_prompt,
                    est_completion, "Using estimated token counts (provider returned no usage)"
                );
                (Some(est_prompt), Some(est_completion))
            },
            |usage| (Some(usage.prompt_tokens), Some(usage.completion_tokens)),
        )
    }

    /// Record the terminal turn-summary row after a chat/insight turn.
    ///
    /// Per-LLM-call token counts are already recorded by the tool
    /// loop's [`crate::services::tool_execution::LlmCallRecorder`]
    /// sink — this row carries only the turn-level aggregates:
    /// `tool_calls_count`, `tools_called`, end-to-end
    /// `execution_time_ms`. Token columns are zeroed so the per-call
    /// rows and the summary row don't double-count in the aggregate
    /// analytics endpoint, which filters by
    /// [`pierre_core::models::TURN_SUMMARY_CALL_TYPE`].
    async fn record_llm_usage(resources: &Arc<ServerResources>, params: &RecordLlmUsageParams<'_>) {
        let tenant_id_str = params.tenant_id.to_string();

        #[allow(clippy::cast_possible_wrap)]
        let exec_time = params.execution_time_ms as i64;

        let tools_called_json = serde_json::to_string(params.tools_called).unwrap_or_else(|e| {
            warn!("Failed to serialize tools_called, storing empty array: {e}");
            "[]".to_owned()
        });

        if let Err(e) = resources
            .repos
            .llm_usage
            .insert_llm_usage(&InsertLlmUsage {
                tenant_id: &tenant_id_str,
                user_id: params.user_id,
                conversation_id: Some(params.conversation_id),
                turn_id: params.turn_id,
                provider: params.provider.name(),
                model: params.model,
                prompt_tokens: 0,
                completion_tokens: 0,
                total_tokens: 0,
                call_type: TURN_SUMMARY_CALL_TYPE,
                tool_calls_count: i64::from(params.tool_calls_count),
                tools_called: &tools_called_json,
                execution_time_ms: Some(exec_time),
            })
            .await
        {
            warn!("Failed to record LLM usage summary: {e}");
        }
    }
}
