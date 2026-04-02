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
use crate::models::ConnectionType;
use crate::models::TenantId;
use crate::protocols::universal::UniversalResponse;
#[cfg(feature = "tools-groups")]
use crate::services::group_fitness::fetch_member_snapshots;
use crate::{
    errors::AppError,
    llm::{
        get_insight_generation_prompt, get_pierre_system_prompt, ChatMessage, ChatProvider,
        FunctionDeclaration, Tool,
    },
    mcp::resources::ServerResources,
    middleware::extract_auth_from_headers,
    protocols::universal::UniversalExecutor,
    services::{
        chat_orchestration,
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
use pierre_core::models::coaches::DataRequirements;
use pierre_core::models::usage::InsertLlmUsage;
use pierre_core::models::AddMessageParams;
use pierre_database::database::{ConversationRecord, MessageRecord};
use pierre_llm::pricing::estimate_tokens;
#[cfg(feature = "client-notifications")]
use pierre_notifications::triggers as notification_triggers;
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, fmt::Write, sync::Arc, time::Instant};
use tracing::{debug, info, warn};
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

/// Strip synthetic function call syntax from LLM content
///
/// Some models (like Llama via Groq) output function calls both as proper `tool_calls`
/// AND as text content using syntax like `<function(name)>{...}</function>` or
/// `<function/name>{...}</function>`.
/// This helper removes that synthetic syntax to avoid displaying it to users.
pub(crate) fn strip_synthetic_function_calls(content: &str) -> Cow<'_, str> {
    use regex::Regex;
    use std::sync::OnceLock;

    fn function_pattern() -> Option<&'static Regex> {
        static PATTERN: OnceLock<Option<Regex>> = OnceLock::new();
        PATTERN
            .get_or_init(|| {
                // Match patterns like:
                // - <function(name)>...</function> (parentheses syntax)
                // - <function/name>...</function> (slash syntax)
                Regex::new(r"<function[/\(][^>]+>[\s\S]*?</function>").ok()
            })
            .as_ref()
    }

    let Some(pattern) = function_pattern() else {
        return Cow::Borrowed(content);
    };

    let cleaned = pattern.replace_all(content, "");
    let trimmed = cleaned.trim();

    if trimmed.is_empty() {
        Cow::Borrowed("")
    } else if trimmed.len() == content.len() {
        Cow::Borrowed(content)
    } else {
        Cow::Owned(trimmed.to_owned())
    }
}

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

/// Extract text content from a `UniversalResponse` for pre-fetched data injection.
///
/// Handles both string results and JSON object results by serializing to
/// a compact representation suitable for LLM context.
fn extract_prefetch_content(response: &UniversalResponse) -> String {
    match &response.result {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(value) => serde_json::to_string(value).unwrap_or_default(),
        None => String::new(),
    }
}

// ============================================================================
// Internal Types
// ============================================================================

/// Parameters for recording LLM usage after a chat completion
struct RecordLlmUsageParams<'a> {
    /// Tenant this usage belongs to
    tenant_id: TenantId,
    /// User who initiated the request
    user_id: &'a str,
    /// Conversation this message belongs to
    conversation_id: &'a str,
    /// LLM provider used for this completion
    provider: &'a ChatProvider,
    /// Model identifier used for this completion
    model: &'a str,
    /// Number of prompt tokens consumed
    prompt_tokens: Option<u32>,
    /// Number of completion tokens generated
    completion_tokens: Option<u32>,
    /// Number of tool calls executed
    tool_calls_count: u32,
    /// Total wall-clock time for the LLM interaction in milliseconds
    execution_time_ms: u64,
    /// Whether this was an insight generation request vs a regular chat
    is_insight_request: bool,
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
    /// System prompt for the conversation (optional)
    #[serde(default)]
    pub system_prompt: Option<String>,
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
    /// System prompt if set
    pub system_prompt: Option<String>,
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

/// Request to send a message
#[derive(Debug, Deserialize)]
pub struct SendMessageRequest {
    /// Message content
    pub content: String,
    /// Whether to stream the response
    #[serde(default)]
    pub stream: bool,
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

    /// Get the system prompt text for a conversation
    ///
    /// Uses conversation-specific prompt if set, otherwise returns the default Pierre system prompt.
    fn get_system_prompt_text(conversation: &ConversationRecord) -> String {
        conversation
            .system_prompt
            .clone()
            .unwrap_or_else(|| get_pierre_system_prompt().to_owned())
    }

    /// Build provider context string for inclusion in system prompt
    ///
    /// Uses `provider_connections` as the single source of truth for which providers
    /// are connected, so the LLM doesn't ask users to connect already-available providers.
    async fn build_provider_context(resources: &Arc<ServerResources>, user_id: Uuid) -> String {
        // Get all provider connections (cross-tenant view, single source of truth)
        let Ok(connections) = resources
            .repos
            .provider_connections
            .get_for_user(user_id, None)
            .await
        else {
            return String::new();
        };

        // Filter out providers that aren't registered in the current runtime
        // (e.g., synthetic providers excluded from production builds)
        let connections: Vec<_> = connections
            .into_iter()
            .filter(|c| resources.provider_registry.is_supported(&c.provider))
            .collect();

        if connections.is_empty() {
            return String::new();
        }

        let mut context = String::from("\n\n## Connected Fitness Data Providers\n\n");
        context.push_str("The user has the following data sources available:\n");
        for conn in &connections {
            let label = if conn.connection_type == ConnectionType::Synthetic {
                Cow::Owned(format!("{} (test data)", conn.provider))
            } else {
                Cow::Borrowed(conn.provider.as_str())
            };
            // Write trait used to avoid format_push_string lint
            let _ = writeln!(context, "- ✓ {label}");
        }
        context.push_str("\nUse the connected providers to fetch activity data. ");
        context
            .push_str("Do NOT ask the user to connect providers that are already connected above.");

        context
    }

    /// Get augmented system prompt with provider context and group coaching context
    async fn get_augmented_system_prompt(
        conversation: &ConversationRecord,
        resources: &Arc<ServerResources>,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> String {
        let base_prompt = Self::get_system_prompt_text(conversation);
        let provider_context = Self::build_provider_context(resources, user_id).await;

        let mut augmented = if provider_context.is_empty() {
            base_prompt
        } else {
            format!("{base_prompt}{provider_context}")
        };

        // Inject group coaching context if the user is in a group with this coach
        #[cfg(feature = "tools-groups")]
        {
            augmented = Self::inject_group_context_if_applicable(
                &augmented,
                conversation,
                resources,
                user_id,
                tenant_id,
            )
            .await;
        }

        augmented
    }

    /// Inject group coaching context into the system prompt if applicable
    #[cfg(feature = "tools-groups")]
    async fn inject_group_context_if_applicable(
        augmented: &str,
        conversation: &ConversationRecord,
        resources: &Arc<ServerResources>,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> String {
        let group_service = resources.group_service();

        // If conversation already has a group_id, use it directly
        let conversation_group_id = conversation.group_id.as_deref();

        // Find groups this user belongs to — for MVP, we check all groups
        // and inject context if the user is in exactly one active group.
        // This works because inject_group_context handles disambiguation.
        let groups = match resources.repos.groups.list_groups_for_user(user_id).await {
            Ok(g) => g,
            Err(e) => {
                tracing::debug!("Failed to check group membership: {e}");
                return augmented.to_owned();
            }
        };

        if groups.is_empty() && conversation_group_id.is_none() {
            return augmented.to_owned();
        }

        // If conversation has no group_id, resolve from user's groups:
        // - 1 group → use it directly (skip coach_id lookup which requires non-empty coach_id)
        // - 2+ groups → let inject_group_context handle disambiguation
        let resolved_group_id = match conversation_group_id {
            Some(id) => Some(id.to_owned()),
            None if groups.len() == 1 => Some(groups[0].id.to_string()),
            _ => None,
        };

        // Collect member user_ids from groups for snapshot fetching
        let member_user_ids: Vec<Uuid> = if let Some(ref gid) = resolved_group_id {
            resources
                .repos
                .groups
                .list_members(gid)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|m| m.user_id)
                .collect()
        } else {
            Vec::new()
        };

        // Fetch fitness snapshots for group members (graceful fallback to empty)
        let snapshots = if member_user_ids.is_empty() {
            Vec::new()
        } else {
            fetch_member_snapshots(resources, &member_user_ids, tenant_id).await
        };

        match group_service
            .inject_group_context(
                augmented,
                "",
                user_id,
                tenant_id,
                resolved_group_id.as_deref(),
                &snapshots,
            )
            .await
        {
            Ok(result) => result,
            Err(e) => {
                tracing::debug!("Group context injection failed: {e}");
                augmented.to_owned()
            }
        }
    }

    /// Get startup context for a coach conversation if applicable.
    ///
    /// Returns the startup query and optionally parsed `DataRequirements` for
    /// deterministic activity pre-fetching.
    ///
    /// Returns `Some((query, data_requirements))` only if:
    /// - This is the first message in the conversation (`history_len == 1`)
    /// - The conversation has a custom `system_prompt` (indicates a coach)
    /// - The coach has a `startup_query` or `data_requirements` configured
    async fn get_startup_context_if_applicable(
        resources: &Arc<ServerResources>,
        history_len: usize,
        system_prompt: Option<&String>,
        tenant_id: TenantId,
    ) -> Option<(Option<String>, Option<DataRequirements>)> {
        // Only inject on first message
        if history_len != 1 {
            return None;
        }

        // Must have a system prompt (indicates coach conversation)
        let prompt = system_prompt?;

        let coaches_manager = resources.coaches_manager();

        match coaches_manager
            .get_startup_context_by_system_prompt(prompt, tenant_id)
            .await
        {
            Ok(Some((query, data_reqs_json))) => {
                if let Some(q) = &query {
                    info!(
                        "Found startup query for coach conversation: {}",
                        &q[..q.len().min(50)]
                    );
                }

                // Parse data_requirements JSON if present
                let data_reqs = data_reqs_json.and_then(|json| {
                    match serde_json::from_str::<DataRequirements>(&json) {
                        Ok(dr) => {
                            info!("Found data_requirements for coach context assembly");
                            Some(dr)
                        }
                        Err(e) => {
                            warn!("Failed to parse data_requirements JSON: {e}");
                            None
                        }
                    }
                });

                Some((query, data_reqs))
            }
            Ok(None) => None,
            Err(e) => {
                warn!("Failed to get startup context: {e}");
                None
            }
        }
    }

    /// Pre-fetch activity data based on structured `DataRequirements`.
    ///
    /// Calls `get_activities` deterministically with exact parameters from the coach
    /// definition, bypassing LLM interpretation. Returns the activity data as a
    /// formatted string for injection into the conversation context.
    async fn prefetch_activity_context(
        executor: &Arc<UniversalExecutor>,
        user_id: &str,
        tenant_id: TenantId,
        data_reqs: &DataRequirements,
    ) -> Option<String> {
        use crate::protocols::universal::handlers::fitness_api::handle_get_activities;
        use crate::protocols::universal::UniversalRequest;

        let activities_req = data_reqs.activities.as_ref()?;

        // Build parameters JSON matching what handle_get_activities expects
        let mut params = serde_json::json!({
            "limit": activities_req.count,
            "mode": activities_req.mode,
            "format": activities_req.format,
            "analysis_type": activities_req.analysis_type,
        });

        // Add sport_type filter if specified (single sport type for now)
        if let Some(sport_type) = activities_req.sport_types.first() {
            params["sport_type"] = serde_json::Value::String(String::clone(sport_type));
        }

        // Add time_frame as 'after' timestamp
        if let Some(seconds) = activities_req.time_frame_seconds() {
            let after = chrono::Utc::now().timestamp() - seconds;
            params["after"] = serde_json::Value::Number(serde_json::Number::from(after));
        }

        let request = UniversalRequest {
            tool_name: "get_activities".to_owned(),
            parameters: params,
            user_id: user_id.to_owned(),
            protocol: "chat".to_owned(),
            tenant_id: Some(tenant_id.to_string()),
            progress_token: None,
            cancellation_token: None,
            progress_reporter: None,
        };

        match handle_get_activities(executor, request).await {
            Ok(response) => {
                let content = extract_prefetch_content(&response);
                info!(
                    content_len = content.len(),
                    "Pre-fetched activity context for coach"
                );
                Some(content)
            }
            Err(e) => {
                warn!("Failed to pre-fetch activity context: {e}");
                None
            }
        }
    }

    /// Inject startup context (pre-fetched data + analysis query) into LLM messages.
    ///
    /// When a coach has `data_requirements`, activity data is fetched deterministically
    /// and injected as system context. The startup query becomes the analysis instruction.
    /// Without `data_requirements`, the startup query is injected as-is for the LLM
    /// to interpret (tool-calling fallback).
    async fn inject_startup_context(
        resources: &Arc<ServerResources>,
        executor: &Arc<UniversalExecutor>,
        llm_messages: &mut Vec<ChatMessage>,
        history: &[MessageRecord],
        system_prompt: Option<&String>,
        user_id_str: &str,
        tenant_id: TenantId,
    ) {
        let Some((startup_query, data_reqs)) = Self::get_startup_context_if_applicable(
            resources,
            history.len(),
            system_prompt,
            tenant_id,
        )
        .await
        else {
            return;
        };

        if let Some(data_reqs) = &data_reqs {
            // Full context assembly: pre-fetch activity data deterministically
            if let Some(activity_context) =
                Self::prefetch_activity_context(executor, user_id_str, tenant_id, data_reqs).await
            {
                let context_msg = format!(
                    "The following activity data has been pre-loaded for your analysis:\n\n\
                     {activity_context}"
                );
                llm_messages.insert(1, ChatMessage::system(&context_msg));
            }

            // Inject the startup query as the analysis instruction (after data context)
            if let Some(query) = &startup_query {
                let insert_pos = llm_messages.len().saturating_sub(1);
                llm_messages.insert(insert_pos, ChatMessage::user(query));
            }
        } else if let Some(query) = &startup_query {
            // No data_requirements: inject startup query for LLM tool-calling
            llm_messages.insert(1, ChatMessage::user(query));
        }
    }

    /// Get LLM provider based on `PIERRE_LLM_PROVIDER` environment variable
    async fn get_llm_provider() -> Result<ChatProvider, AppError> {
        super::create_chat_provider().await
    }

    /// Build LLM messages from conversation history and optional system prompt
    fn build_llm_messages(
        system_prompt: Option<&str>,
        history: &[MessageRecord],
    ) -> Vec<ChatMessage> {
        let mut messages = Vec::with_capacity(history.len() + 1);

        if let Some(prompt) = system_prompt {
            messages.push(ChatMessage::system(prompt));
        }

        for msg in history {
            let chat_msg = match msg.role.as_str() {
                "user" => ChatMessage::user(&msg.content),
                "assistant" => ChatMessage::assistant(&msg.content),
                "system" => ChatMessage::system(&msg.content),
                _ => continue,
            };
            messages.push(chat_msg);
        }

        messages
    }

    /// Build connection-related tool definitions
    fn build_connection_tools() -> Vec<FunctionDeclaration> {
        vec![
            FunctionDeclaration {
                name: "get_connection_status".to_owned(),
                description: "Check which fitness providers are connected".to_owned(),
                parameters: Some(serde_json::json!({"type": "object", "properties": {}})),
            },
            FunctionDeclaration {
                name: "connect_provider".to_owned(),
                description: "Connect to a fitness provider via OAuth".to_owned(),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {"provider": {"type": "string"}},
                    "required": ["provider"]
                })),
            },
            FunctionDeclaration {
                name: "disconnect_provider".to_owned(),
                description: "Disconnect a fitness provider".to_owned(),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {"provider": {"type": "string"}},
                    "required": ["provider"]
                })),
            },
        ]
    }

    /// Build activity data tool definitions
    fn build_activity_tools() -> Vec<FunctionDeclaration> {
        vec![
            FunctionDeclaration {
                name: "get_activities".to_owned(),
                description: "Get user's recent fitness activities".to_owned(),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "provider": {"type": "string"},
                        "limit": {"type": "integer"},
                        "offset": {"type": "integer"}
                    },
                    "required": ["provider"]
                })),
            },
            FunctionDeclaration {
                name: "get_athlete".to_owned(),
                description: "Get user's athlete profile information".to_owned(),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {"provider": {"type": "string"}},
                    "required": ["provider"]
                })),
            },
            FunctionDeclaration {
                name: "get_stats".to_owned(),
                description: "Get user's overall fitness statistics".to_owned(),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {"provider": {"type": "string"}},
                    "required": ["provider"]
                })),
            },
        ]
    }

    /// Build analysis tool definitions
    fn build_analysis_tools() -> Vec<FunctionDeclaration> {
        vec![
            FunctionDeclaration {
                name: "analyze_activity".to_owned(),
                description: "Deep analysis of a specific activity".to_owned(),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "provider": {"type": "string"},
                        "activity_id": {"type": "string"}
                    },
                    "required": ["provider", "activity_id"]
                })),
            },
            FunctionDeclaration {
                name: "get_activity_intelligence".to_owned(),
                description: "AI-powered insights including location and weather".to_owned(),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "provider": {"type": "string"},
                        "activity_id": {"type": "string"},
                        "include_location": {"type": "boolean"},
                        "include_weather": {"type": "boolean"}
                    },
                    "required": ["provider", "activity_id"]
                })),
            },
            FunctionDeclaration {
                name: "analyze_performance_trends".to_owned(),
                description: "Analyze performance trends over time".to_owned(),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "provider": {"type": "string"},
                        "timeframe": {"type": "string"},
                        "metric": {"type": "string"},
                        "sport_type": {"type": "string"}
                    },
                    "required": ["provider", "timeframe", "metric"]
                })),
            },
            FunctionDeclaration {
                name: "compare_activities".to_owned(),
                description: "Compare activity against similar or personal bests".to_owned(),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "provider": {"type": "string"},
                        "activity_id": {"type": "string"},
                        "comparison_type": {"type": "string"}
                    },
                    "required": ["provider", "activity_id", "comparison_type"]
                })),
            },
            FunctionDeclaration {
                name: "calculate_fitness_score".to_owned(),
                description: "Calculate comprehensive fitness score".to_owned(),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "provider": {"type": "string"},
                        "timeframe": {"type": "string"},
                        "sleep_provider": {"type": "string"}
                    },
                    "required": ["provider"]
                })),
            },
            FunctionDeclaration {
                name: "analyze_training_load".to_owned(),
                description: "Analyze training load and recovery needs".to_owned(),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "provider": {"type": "string"},
                        "timeframe": {"type": "string"},
                        "sleep_provider": {"type": "string"}
                    },
                    "required": ["provider"]
                })),
            },
        ]
    }

    /// Build recovery and recommendation tool definitions
    fn build_recovery_tools() -> Vec<FunctionDeclaration> {
        vec![
            FunctionDeclaration {
                name: "calculate_recovery_score".to_owned(),
                description: "Calculate recovery score with daily strain (WHOOP cycles), HRV, sleep quality, and TSB. Use when user asks about recovery, daily strain, WHOOP cycles, or training readiness.".to_owned(),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "activity_provider": {"type": "string"},
                        "sleep_provider": {"type": "string"}
                    }
                })),
            },
            FunctionDeclaration {
                name: "suggest_rest_day".to_owned(),
                description: "AI recommendation for rest day".to_owned(),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "activity_provider": {"type": "string"},
                        "sleep_provider": {"type": "string"}
                    }
                })),
            },
            FunctionDeclaration {
                name: "generate_recommendations".to_owned(),
                description: "Get personalized training recommendations".to_owned(),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "provider": {"type": "string"},
                        "recommendation_type": {"type": "string"},
                        "activity_id": {"type": "string"}
                    },
                    "required": ["provider"]
                })),
            },
        ]
    }

    /// Build Gemini tool definitions from MCP tool registry
    pub(crate) fn build_mcp_tools() -> Tool {
        let mut declarations = Vec::with_capacity(14);
        declarations.extend(Self::build_connection_tools());
        declarations.extend(Self::build_activity_tools());
        declarations.extend(Self::build_analysis_tools());
        declarations.extend(Self::build_recovery_tools());
        Tool {
            function_declarations: declarations,
        }
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
            request.system_prompt.as_deref(),
        )
        .await?;

        let conv = result.conversation;
        let response = ConversationResponse {
            id: conv.id,
            title: conv.title,
            model: conv.model,
            system_prompt: conv.system_prompt,
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
            system_prompt: conv.system_prompt,
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
            system_prompt: conv.system_prompt,
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

    /// Send a message and get a response (non-streaming) with MCP tool execution
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

        // Pre-chat quota check: verify message and token quotas before LLM dispatch
        let usage_warning = Self::check_pre_chat_quotas(
            &resources,
            &tenant_id_str,
            &user_id_str,
            auth.user_id,
            tenant_id,
        )
        .await?;

        // Verify ownership and persist user message (crash-safe: saved before LLM dispatch)
        let msg_result = chat_orchestration::persist_user_message(
            resources.repos.chat.as_ref(),
            &conversation_id,
            &user_id_str,
            tenant_id,
            &request.content,
        )
        .await?;

        let conv = msg_result.conversation;
        let user_msg = msg_result.message;

        // Get conversation history for LLM context
        let history = chat_orchestration::get_conversation_history(
            resources.repos.chat.as_ref(),
            &conversation_id,
            &user_id_str,
        )
        .await?;

        // Check if this is an insight generation request
        // These use a dedicated prompt optimized for clean, shareable output
        let is_insight_request = request.content.starts_with(INSIGHT_PROMPT_PREFIX);

        let mut llm_messages = if is_insight_request {
            // For insight generation: use dedicated prompt and extract just the analysis
            let insight_prompt = get_insight_generation_prompt();

            // Extract the analysis content (everything after the prefix and colon/newlines)
            let analysis_content = request
                .content
                .strip_prefix(INSIGHT_PROMPT_PREFIX)
                .unwrap_or(&request.content)
                .trim_start_matches(':')
                .trim();

            // Build messages without history - insight generation is a single-turn task
            vec![
                ChatMessage::system(insight_prompt),
                ChatMessage::user(analysis_content),
            ]
        } else {
            // Normal conversation: use augmented system prompt with full history
            let system_prompt_text =
                Self::get_augmented_system_prompt(&conv, &resources, auth.user_id, tenant_id).await;
            Self::build_llm_messages(Some(system_prompt_text.as_str()), &history)
        };

        // Create MCP executor for tool calls (Arc for sharing with SDK tool handler closures)
        // Created early because prefetch_activity_context needs it
        let executor = Arc::new(UniversalExecutor::new(resources.clone()));

        // Inject startup context if this is the first message in a coach conversation
        if !is_insight_request {
            Self::inject_startup_context(
                &resources,
                &executor,
                &mut llm_messages,
                &history,
                conv.system_prompt.as_ref(),
                &user_id_str,
                tenant_id,
            )
            .await;
        }

        // Build MCP tools for function calling
        let tools = Self::build_mcp_tools();

        // Get LLM provider
        let provider = Self::get_llm_provider().await?;

        // Resolve max tool iterations: coach setting > admin config > default
        let max_iterations =
            Self::resolve_max_tool_iterations(&resources, conv.system_prompt.as_ref(), tenant_id)
                .await;

        // Track execution time for the entire LLM + tool loop
        let start_time = Instant::now();

        // Run multi-turn tool execution loop
        let tool_params = ToolLoopParams {
            provider: &provider,
            executor: Arc::clone(&executor),
            tools: &tools,
            model: &conv.model,
            user_id: &user_id_str,
            tenant_id,
            max_iterations,
        };
        let result = chat_tool_loop::run_tool_loop(&tool_params, &mut llm_messages).await?;

        info!(
            content_len = result.content.len(),
            content_preview = %result.content.chars().take(300).collect::<String>(),
            tool_calls = result.tool_calls_count,
            finish_reason = ?result.finish_reason,
            has_usage = result.usage.is_some(),
            "Tool loop completed"
        );

        // Safe cast: execution time will never exceed u64::MAX milliseconds (~584 million years)
        #[allow(clippy::cast_possible_truncation)]
        let execution_time_ms = start_time.elapsed().as_millis() as u64;

        // Extract token counts from usage, falling back to character-based estimation
        // when the provider doesn't return real counts (e.g., CLI-based providers)
        let (prompt_tokens, token_count) = Self::extract_or_estimate_tokens(&result, &llm_messages);

        // Process content: parse insight JSON if applicable
        let final_content = Self::post_process_content(&result.content, is_insight_request);

        // Persist assistant response with full token usage and model name
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
        let (assistant_msg, updated_conv) = chat_orchestration::persist_assistant_response(
            resources.repos.chat.as_ref(),
            &assistant_params,
            tenant_id,
        )
        .await?;

        // Record LLM usage for cost tracking and quota enforcement
        Self::record_llm_usage(
            &resources,
            &RecordLlmUsageParams {
                tenant_id,
                user_id: &user_id_str,
                conversation_id: &conversation_id,
                provider: &provider,
                model: &conv.model,
                prompt_tokens,
                completion_tokens: token_count,
                tool_calls_count: result.tool_calls_count,
                execution_time_ms,
                is_insight_request,
            },
        )
        .await;

        // Increment usage counters after successful LLM call
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
        };

        // Notify user when a coach conversation produces a response
        #[cfg(feature = "client-notifications")]
        Self::notify_coach_response(&resources, &conv, auth.user_id, tenant_id, &conversation_id);

        // Build response with usage warning headers
        let mut http_response = (StatusCode::OK, Json(response)).into_response();
        Self::apply_usage_warning_headers(&mut http_response, usage_warning);

        Ok(http_response)
    }

    /// Fire-and-forget notification when a coach conversation produces a response.
    /// Only sends if the conversation has a `system_prompt` (indicates coach persona).
    #[cfg(feature = "client-notifications")]
    fn notify_coach_response(
        resources: &Arc<ServerResources>,
        conv: &ConversationRecord,
        user_id: Uuid,
        tenant_id: TenantId,
        conversation_id: &str,
    ) {
        if conv.system_prompt.is_some() {
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

    /// Resolve the maximum tool iterations for this conversation
    ///
    /// Resolution hierarchy: coach `max_tool_iterations` > admin config > default (10).
    /// Looks up the coach by system prompt when the conversation has one.
    async fn resolve_max_tool_iterations(
        resources: &Arc<ServerResources>,
        system_prompt: Option<&String>,
        tenant_id: TenantId,
    ) -> usize {
        // Try coach-level override via system prompt lookup
        if let Some(prompt) = system_prompt {
            {
                let coaches_manager = resources.coaches_manager();
                if let Ok(Some(iterations)) = coaches_manager
                    .get_max_tool_iterations_by_system_prompt(prompt, tenant_id)
                    .await
                {
                    #[allow(clippy::cast_sign_loss)]
                    let value = iterations.max(1) as usize;
                    debug!(
                        max_tool_iterations = value,
                        "Using coach-level tool iteration limit"
                    );
                    return value;
                }
            }
        }

        // Try admin config override
        if let Some(ref admin_config) = resources.admin_config {
            if let Ok(Some(val)) = admin_config
                .get_value("tool_execution.max_iterations", None)
                .await
            {
                if let Some(config_val) = val.as_i64() {
                    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                    let value = (config_val.max(1) as usize).min(50);
                    debug!(
                        max_tool_iterations = value,
                        "Using admin config tool iteration limit"
                    );
                    return value;
                }
            }
        }

        DEFAULT_MAX_TOOL_ITERATIONS
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
                let (est_prompt, est_completion) = estimate_tokens(&prompt_text, &result.content);
                debug!(
                    est_prompt,
                    est_completion, "Using estimated token counts (provider returned no usage)"
                );
                (Some(est_prompt), Some(est_completion))
            },
            |usage| (Some(usage.prompt_tokens), Some(usage.completion_tokens)),
        )
    }

    /// Record LLM usage after chat completion for cost tracking and quota enforcement
    async fn record_llm_usage(resources: &Arc<ServerResources>, params: &RecordLlmUsageParams<'_>) {
        let tenant_id_str = params.tenant_id.to_string();
        let prompt_count = i64::from(params.prompt_tokens.unwrap_or(0));
        let completion_count = i64::from(params.completion_tokens.unwrap_or(0));
        let call_type = if params.is_insight_request {
            "insight"
        } else {
            "chat"
        };

        #[allow(clippy::cast_possible_wrap)]
        let exec_time = params.execution_time_ms as i64;

        if let Err(e) = resources
            .repos
            .llm_usage
            .insert_llm_usage(&InsertLlmUsage {
                tenant_id: &tenant_id_str,
                user_id: params.user_id,
                conversation_id: Some(params.conversation_id),
                provider: params.provider.name(),
                model: params.model,
                prompt_tokens: prompt_count,
                completion_tokens: completion_count,
                total_tokens: prompt_count + completion_count,
                call_type,
                tool_calls_count: i64::from(params.tool_calls_count),
                execution_time_ms: Some(exec_time),
            })
            .await
        {
            warn!("Failed to record LLM usage: {e}");
        }
    }
}
