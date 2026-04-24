// ABOUTME: Request and response DTOs shared across chat route handlers
// ABOUTME: Kept in one file so conversations, send_message, send_insight can import consistent shapes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde::{Deserialize, Serialize};

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
    /// Optional card title for command responses (e.g. `/coach` → "Choose a coach").
    /// Present only when the assistant reply came from a slash-command handler that
    /// returned a card shape; absent for regular LLM turns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub card_title: Option<String>,
    /// Optional action buttons for command responses (e.g. per-coach select buttons).
    /// Frontends render these as clickable buttons whose click re-POSTs the action's
    /// `value` (e.g. `/coach select <uuid>`) as the user's next message, flowing back
    /// through the same dispatch pipeline.
    ///
    /// Not persisted — exists only on the turn that produced them. Historical
    /// messages show the rendered text body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<ChatMessageAction>>,
    /// When `true`, the assistant response came from a local slash-command
    /// handler rather than the LLM. Frontends can skip the usual
    /// "LLM-generated" caveats/UI treatment on these turns.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_command_response: bool,
    /// AG-UI `run_id` echoed back when the request supplied one. The
    /// caller uses it to correlate this turn with its parallel
    /// `/api/agui/runs/{run_id}/stream` subscription.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agui_run_id: Option<String>,
}

/// Interactive button attached to a command-response turn.
///
/// Mirrors the `CommandAction` shape from `pierre-messaging` but lives in
/// the HTTP DTO layer so frontends don't need to import messaging types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessageAction {
    /// User-visible button label.
    pub label: String,
    /// Action kind: `"postback"` (re-POST `value` as the next message) or
    /// `"url"` (open `value` in a browser). Other types are ignored by
    /// frontends.
    pub action_type: String,
    /// For `postback`: the text to send as the next user message (e.g.
    /// `/coach select <uuid>`). For `url`: the absolute URL to open.
    pub value: String,
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
