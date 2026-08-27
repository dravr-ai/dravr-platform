// ABOUTME: Request and response DTOs shared across chat route handlers
// ABOUTME: Kept in one file so conversations and send_message import consistent shapes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use photograveur::{resolve_all, Locale};
use pierre_chat_pipeline::stages::viz_blocks::strip_markers;
use pierre_core::models::{
    ConversationParticipant, ParticipantRole, PersistedReplyBlock, ACTIONS_BLOCK_TYPE,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

/// What a stored `content_blocks` column resolves to on the read path.
#[derive(Debug, Default)]
pub struct StoredBlocks {
    /// Resolved visual scenes, JSON-encoded — see [`MessageResponse::scene_blocks`].
    pub scene_blocks: Option<String>,
    /// The controls a persisted command reply carried.
    pub actions: Option<MessageActionsResponse>,
}

/// Resolve a stored `content_blocks` column into what a client renders.
///
/// `stored` is the JSON array persisted on `chat_messages.content_blocks`. It
/// holds two kinds of entry: the visual specs the coach wrote, resolved by
/// photograveur on every read (so a geometry improvement reaches charts already
/// sitting in history without a migration), and the controls a slash-command
/// reply carried, which are partitioned out first — photograveur must never be
/// handed a `{"type":"actions"}` entry as if it were a chart.
///
/// A visual that fails to resolve is dropped and logged rather than failing
/// the message: one malformed chart must never cost the athlete the reply
/// carrying it. Absent parts are `None`, so the fields are omitted from the
/// response entirely.
#[must_use]
pub fn resolve_stored_blocks(stored: Option<&str>, locale: &str) -> StoredBlocks {
    let Some(entries) = stored.and_then(parse_stored_specs) else {
        return StoredBlocks::default();
    };
    let (actions, specs): (Vec<Value>, Vec<Value>) = entries
        .into_iter()
        .partition(|entry| entry.get("type").and_then(Value::as_str) == Some(ACTIONS_BLOCK_TYPE));
    StoredBlocks {
        scene_blocks: resolve_visual_specs(&specs, locale),
        actions: actions.into_iter().find_map(decode_actions_entry),
    }
}

/// Resolve stored visual specs into renderable scenes — the visual half of
/// [`resolve_stored_blocks`], for the live turn whose specs never carry
/// controls.
#[must_use]
pub fn resolve_scene_blocks(stored: Option<&str>, locale: &str) -> Option<String> {
    let specs = parse_stored_specs(stored?)?;
    resolve_visual_specs(&specs, locale)
}

/// Run photograveur over the visual specs, dropping and logging the ones that
/// cannot resolve.
fn resolve_visual_specs(specs: &[Value], locale: &str) -> Option<String> {
    if specs.is_empty() {
        return None;
    }
    let (blocks, failures) = resolve_all(specs, Locale::from_tag(locale));
    for (index, error) in &failures {
        warn!(index, error = %error, "scene-blocks: dropping a block that could not resolve");
    }
    if blocks.is_empty() {
        return None;
    }
    encode_blocks(&blocks)
}

/// Decode one `{"type":"actions"}` entry, logging and skipping a malformed one.
fn decode_actions_entry(entry: Value) -> Option<MessageActionsResponse> {
    match serde_json::from_value::<PersistedReplyBlock>(entry) {
        Ok(PersistedReplyBlock::Actions { title, actions }) => Some(MessageActionsResponse {
            title,
            actions: actions
                .into_iter()
                .map(|action| ChatMessageAction {
                    label: action.label,
                    action_type: action.action_type,
                    value: action.value,
                })
                .collect(),
        }),
        Err(e) => {
            warn!(error = %e, "scene-blocks: stored actions entry is malformed; omitting it");
            None
        }
    }
}

/// Longest preview a list row carries, in characters.
const PREVIEW_CHARS: usize = 120;

/// Shape the newest row's content into a one-line preview.
///
/// Visual markers go (a `⟦viz:0⟧` is a bug in a list row), whitespace runs
/// collapse to one space so a markdown reply reads as one line, and the
/// result is cut at [`PREVIEW_CHARS`] characters — a boundary that never
/// splits a multi-byte character.
#[must_use]
pub fn preview_text(content_head: &str) -> String {
    let stripped = strip_markers(content_head);
    let mut preview = String::with_capacity(stripped.len().min(PREVIEW_CHARS * 4));
    for word in stripped.split_whitespace() {
        if !preview.is_empty() {
            preview.push(' ');
        }
        preview.push_str(word);
    }
    preview.chars().take(PREVIEW_CHARS).collect()
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
    /// One page, newest activity first
    pub conversations: Vec<ConversationSummaryResponse>,
    /// How many conversations the caller is in altogether — the number to
    /// page against, not the page length.
    pub total: i64,
}

/// One row of the unified conversation list.
#[derive(Debug, Serialize, Deserialize)]
pub struct ConversationSummaryResponse {
    /// Conversation ID
    pub id: String,
    /// Conversation title
    pub title: String,
    /// Model used
    pub model: String,
    /// Number of `user` and `assistant` turns; tool rows are not counted.
    pub message_count: i64,
    /// Total tokens used
    pub total_tokens: i64,
    /// Coach attached to the conversation, if any.
    pub coach_id: Option<String>,
    /// The attached coach's catalogue `@handle`, when it has one.
    #[serde(default)]
    pub coach_handle: Option<String>,
    /// The attached coach's title, when the coach still exists.
    #[serde(default)]
    pub coach_title: Option<String>,
    /// Coaching group the conversation is scoped to, if any.
    #[serde(default)]
    pub group_id: Option<String>,
    /// That group's name, when the group still exists.
    #[serde(default)]
    pub group_name: Option<String>,
    /// Channel of origin (`web`/`mobile` for in-app, `telegram`/`whatsapp`/…
    /// for messaging). The client prefers this durable signal for the channel
    /// badge and falls back to parsing the `Messaging: <channel>` title.
    pub channel_type: Option<String>,
    /// The newest `user`/`assistant` row, shaped for the row preview; absent
    /// for an empty conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message: Option<LastMessageResponse>,
    /// `user`/`assistant` rows the caller has not read — every row when they
    /// have never opened the thread.
    #[serde(default)]
    pub unread_count: i64,
    /// Creation timestamp
    pub created_at: String,
    /// Last update timestamp
    pub updated_at: String,
}

/// The newest row of a conversation, as a list row shows it.
#[derive(Debug, Serialize, Deserialize)]
pub struct LastMessageResponse {
    /// One line of the row's content: visual markers stripped, whitespace
    /// collapsed, at most 120 characters.
    pub preview: String,
    /// `user` or `assistant`.
    pub role: String,
    /// When the row was written
    pub created_at: String,
}

/// Request to update a conversation title
#[derive(Debug, Deserialize)]
pub struct UpdateConversationRequest {
    /// New title
    pub title: String,
}

/// Request to add a participant to a conversation
#[derive(Debug, Deserialize)]
pub struct AddParticipantRequest {
    /// The user to add. Must be a member of the conversation's tenant.
    pub user_id: String,
}

/// One participant of a conversation
#[derive(Debug, Serialize, Deserialize)]
pub struct ParticipantResponse {
    /// The participating user
    pub user_id: String,
    /// `owner` or `member`
    pub role: ParticipantRole,
    /// The participant who added this one (the owner names themself)
    pub added_by: String,
    /// When the membership was written
    pub added_at: String,
}

impl From<ConversationParticipant> for ParticipantResponse {
    fn from(p: ConversationParticipant) -> Self {
        Self {
            user_id: p.user_id,
            role: p.role,
            added_by: p.added_by,
            added_at: p.added_at,
        }
    }
}

/// Response for listing a conversation's participants
#[derive(Debug, Serialize, Deserialize)]
pub struct ParticipantListResponse {
    /// Owner first, then members in the order they were added
    pub participants: Vec<ParticipantResponse>,
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
    /// Why this row ended: the provider's own reason for an LLM row, or one
    /// of the platform's stamps — `command` marks a slash-command turn (both
    /// the `/…` line and its answer), the field a client already reads on a
    /// live turn to tell a command from a coaching reply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    /// The controls a persisted command reply carried, so a reload draws the
    /// same buttons the live turn did. Absent on a live turn, whose controls
    /// ride the block list instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<MessageActionsResponse>,
    /// Creation timestamp
    pub created_at: String,
}

/// The controls persisted with a command reply.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageActionsResponse {
    /// Label for the group, e.g. a picker's card title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The controls, in order.
    pub actions: Vec<ChatMessageAction>,
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
    /// `/coach add @handle`). For `url`: the absolute URL to open.
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
    /// the send path that builds `MessageResponse` needs no change.
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
#[derive(Debug, Deserialize)]
pub struct ListConversationsQuery {
    /// Maximum number of conversations to return, clamped between
    /// [`MIN_LIST_LIMIT`] and [`MAX_LIST_LIMIT`]
    #[serde(default = "default_limit")]
    pub limit: i64,
    /// Offset for pagination; a negative value reads as the first page
    #[serde(default)]
    pub offset: i64,
}

impl Default for ListConversationsQuery {
    fn default() -> Self {
        Self {
            limit: default_limit(),
            offset: 0,
        }
    }
}

/// Smallest page a client may ask for.
pub const MIN_LIST_LIMIT: i64 = 1;
/// Largest page a client may ask for — a whole list for an active athlete,
/// without letting one call pull every conversation a tenant holds.
pub const MAX_LIST_LIMIT: i64 = 200;

const fn default_limit() -> i64 {
    50
}

impl ListConversationsQuery {
    /// The page bounds actually applied: `limit` clamped into range, a
    /// negative `offset` read as zero.
    #[must_use]
    pub fn bounded(&self) -> (i64, i64) {
        (
            self.limit.clamp(MIN_LIST_LIMIT, MAX_LIST_LIMIT),
            self.offset.max(0),
        )
    }
}
