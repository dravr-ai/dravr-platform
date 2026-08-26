// ABOUTME: Request and response DTOs shared across chat route handlers
// ABOUTME: Kept in one file so conversations, send_message, send_insight can import consistent shapes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use photograveur::{resolve_all, Locale};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

/// Resolve stored visual specs into renderable scenes.
///
/// `stored` is the JSON array persisted on `chat_messages.content_blocks` — the
/// specs the coach wrote, which are the durable record. This runs on every read
/// rather than at write time, so improving the geometry engine improves charts
/// already sitting in conversation history without a migration.
///
/// A block that fails to resolve is dropped and logged rather than failing the
/// message: one malformed chart must never cost the athlete the reply carrying
/// it. Returns `None` when there is nothing to render, so the field is omitted
/// from the response entirely.
#[must_use]
pub fn resolve_scene_blocks(stored: Option<&str>, locale: &str) -> Option<String> {
    let specs = parse_stored_specs(stored?)?;

    let (blocks, failures) = resolve_all(&specs, Locale::from_tag(locale));
    for (index, error) in &failures {
        warn!(index, error = %error, "scene-blocks: dropping a block that could not resolve");
    }
    if blocks.is_empty() {
        return None;
    }
    encode_blocks(&blocks)
}

/// Parse the stored spec array, logging and yielding `None` on malformed JSON.
fn parse_stored_specs(raw: &str) -> Option<Vec<Value>> {
    serde_json::from_str(raw)
        .inspect_err(
            |e| warn!(error = %e, "scene-blocks: stored blocks are not a JSON array; omitting them"),
        )
        .ok()
}

/// Encode resolved blocks for the wire, logging and yielding `None` on failure.
fn encode_blocks(blocks: &[photograveur::RenderBlock]) -> Option<String> {
    serde_json::to_string(blocks)
        .inspect_err(
            |e| warn!(error = %e, "scene-blocks: resolved scenes failed to serialize; omitting them"),
        )
        .ok()
}

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
    /// Coaching group ID to scope this conversation to (optional). When
    /// set, the server-side prompt assembly stage injects group context
    /// (member roster, peer training data with consent, role-aware
    /// summaries). The caller must be an active member of the group.
    #[serde(default)]
    pub group_id: Option<String>,
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
    /// Coaching group attached to this conversation, if any
    #[serde(default)]
    pub group_id: Option<String>,
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
    /// Coach attached to the conversation, if any. Lets the client group
    /// sessions by coach and show the coach's name in the header/history.
    pub coach_id: Option<String>,
    /// Channel of origin (`web`/`mobile` for in-app, `telegram`/`whatsapp`/…
    /// for messaging). The client prefers this durable signal for the channel
    /// badge and falls back to parsing the `Messaging: <channel>` title.
    pub channel_type: Option<String>,
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
#[derive(Debug, Clone, Deserialize)]
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
    /// Ordered visual blocks resolved for rendering, JSON-encoded array of
    /// photograveur `RenderBlock`. The content carries a `⟦viz:N⟧` marker where
    /// each block sat; clients split on the markers and interleave rendering.
    ///
    /// This is the *resolved* form, not the spec the coach wrote. The spec stays
    /// on the message row and the scene is recomputed here on every read, so a
    /// geometry improvement reaches charts already sitting in history without a
    /// migration. Clients therefore never see chart maths — a scene is a flat
    /// list of positioned primitives they map to SVG.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scene_blocks: Option<String>,
    /// Creation timestamp
    pub created_at: String,
}

/// Interactive control attached to a turn.
///
/// The wire form of [`pierre_chat_pipeline::TurnAction`], carried inside an
/// `actions` reply block. Lives in the HTTP DTO layer so frontends don't need
/// to import pipeline types.
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
    /// The caller's own thumbs up/down feedback on these messages, keyed by
    /// `message_id`; empty when none has been left.
    ///
    /// Kept parallel to `messages` (rather than nested on each
    /// `MessageResponse`) so clients hydrate their feedback map directly and
    /// the send/insight paths that build `MessageResponse` need no change.
    pub feedback: Vec<MessageFeedbackEntry>,
}

/// A thumbs up/down rating value for a chat message.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackRating {
    /// 👍 — the reply was helpful.
    Up,
    /// 👎 — the reply was poor.
    Down,
}

impl FeedbackRating {
    /// The persisted string form (`"up"` / `"down"`).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

/// Request to set the caller's feedback on a message.
#[derive(Debug, Deserialize)]
pub struct UpsertFeedbackRequest {
    /// Thumbs up or down.
    pub rating: FeedbackRating,
    /// Optional free-text "what went wrong?" reason — typically only sent
    /// alongside a thumbs-down. Trimmed empty strings are treated as absent.
    #[serde(default)]
    pub comment: Option<String>,
}

/// The caller's feedback on one message, returned inline with the messages
/// list and echoed back from the upsert endpoint.
#[derive(Debug, Serialize, Deserialize)]
pub struct MessageFeedbackEntry {
    /// Message the feedback is attached to.
    pub message_id: String,
    /// Rating value: `"up"` or `"down"`.
    pub rating: String,
    /// Optional free-text reason captured on a thumbs-down.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
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
