// ABOUTME: Chat conversation and message record types for database persistence
// ABOUTME: DTOs for multi-tenant chat conversations with LLM model tracking
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;

use dravr_canot::turn::ConversationTurnId as CanotTurnId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::coaches::CoachCategory;
use crate::models::tenant::TenantId;

/// Identifier for a single conversation turn.
///
/// A *turn* is one inbound user utterance plus the full chain of LLM
/// calls, tool invocations, and the resulting reply. Every component
/// that participates in that chain carries the same
/// [`ConversationTurnId`], which lets per-turn observability queries
/// (cost, latency, tools called) attribute every record to the
/// originating utterance.
///
/// The identifier is generated **once** at the inbound boundary — a
/// messaging webhook, a `/api/chat` request, or a CLI entry point —
/// and propagated through every downstream call. Downstream
/// components must never regenerate it.
///
/// Wire format is a plain UUID string, which keeps the type
/// byte-compatible with the structurally-identical newtypes defined
/// in sibling crates (`dravr-canot::turn::ConversationTurnId`,
/// `embacle::turn::ConversationTurnId`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "sqlx-types", derive(sqlx::Type))]
#[cfg_attr(feature = "sqlx-types", sqlx(transparent))]
#[serde(transparent)]
pub struct ConversationTurnId(pub Uuid);

impl ConversationTurnId {
    /// Generate a new random turn identifier.
    ///
    /// Only inbound boundaries should call this. Downstream callers
    /// propagate the identifier they received.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Wrap an existing UUID as a turn identifier.
    #[must_use]
    pub const fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    /// Return the underlying UUID.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }

    /// Sentinel value used for rows that pre-date turn threading.
    ///
    /// Stored as the nil UUID. Documented in the migration history
    /// so operators can distinguish "unknown" from "genuine turn".
    #[must_use]
    pub const fn nil() -> Self {
        Self(Uuid::nil())
    }
}

impl Default for ConversationTurnId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ConversationTurnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<Uuid> for ConversationTurnId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl From<ConversationTurnId> for Uuid {
    fn from(value: ConversationTurnId) -> Self {
        value.0
    }
}

// Bidirectional conversions with dravr-canot's structurally-identical
// newtype. Each crate owns its own `ConversationTurnId` so it does not
// depend on this module's shape, but their wire format is a plain UUID
// string so the mapping is lossless. Platform code that builds an
// `OutgoingMessage` (canot type) from a pipeline turn id (pierre-core
// type) relies on these impls.
impl From<CanotTurnId> for ConversationTurnId {
    fn from(value: CanotTurnId) -> Self {
        Self(value.as_uuid())
    }
}

impl From<ConversationTurnId> for CanotTurnId {
    fn from(value: ConversationTurnId) -> Self {
        Self::from_uuid(value.0)
    }
}

/// Database representation of a chat conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationRecord {
    /// Unique conversation ID
    pub id: String,
    /// User ID who owns the conversation
    pub user_id: String,
    /// Tenant ID for multi-tenant isolation
    pub tenant_id: String,
    /// Conversation title (auto-generated or user-defined)
    pub title: String,
    /// LLM model used for this conversation
    pub model: String,
    /// Coach that owns this conversation's persona, if any. The coach's
    /// `system_prompt` is resolved at runtime from the `coaches` table.
    #[serde(default)]
    pub coach_id: Option<String>,
    /// Long-lived coach session this conversation participates in
    /// (Tier 4 cross-channel continuity). Resolved on first turn for
    /// conversations that have a `coach_id`.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Total tokens used in this conversation
    pub total_tokens: i64,
    /// When the conversation was created (ISO 8601)
    pub created_at: String,
    /// When the conversation was last updated (ISO 8601)
    pub updated_at: String,
    /// Optional coaching group context for group-scoped conversations
    #[serde(default)]
    pub group_id: Option<String>,
    /// Active pillar-onboarding flow state (JSON). When set, prompt assembly
    /// runs this conversation in guided onboarding mode and the extraction
    /// worker stamps captured facts with `source=onboarding`. `None` for
    /// normal coaching conversations.
    #[serde(default)]
    pub onboarding_state: Option<String>,
}

/// Runtime context for a coach attached to a conversation.
///
/// Consolidates the handful of coach fields the chat pipeline needs on every
/// turn (system prompt, startup context, tool-iteration override) into a
/// single tenant-scoped lookup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoachRuntimeContext {
    /// Stable slug identifier for the coach (matches the contremaitre
    /// markdown filename without `.md`). Used to look up the live coach
    /// prompt in `PromptRegistry` when `source == "contremaitre"`.
    pub slug: String,
    /// Origin of this coach: `"contremaitre"` (git-managed, hot-reloaded
    /// from the dravr-contremaitre repo), `"seed"` (legacy seeded rows
    /// from before the contremaitre migration), or `"custom"` (user/admin
    /// authored — DB-only, no registry overlay). The prompt-assembly
    /// stage consults [`crate::contremaitre::PromptRegistry`] for
    /// `"contremaitre"` rows so a contremaitre prompt edit appears in the
    /// next chat turn without a seeder re-run.
    pub source: String,
    /// The coach's system prompt text from the database column. Acts as
    /// the cold-start fallback when the registry has no entry for
    /// `(slug, locale)`.
    pub system_prompt: String,
    /// Optional startup query the coach wants injected on the first turn
    pub startup_query: Option<String>,
    /// Optional JSON-encoded data requirements for deterministic pre-fetch
    pub data_requirements: Option<String>,
    /// Optional structured-output schema identifier (e.g. `"structured-workout"`).
    /// When set, the pipeline appends the structured-output contract to the
    /// system prompt and extracts/validates/renders the emitted plan JSON.
    pub output_schema: Option<String>,
    /// Optional per-coach override for max tool-call iterations per turn
    pub max_tool_iterations: Option<i32>,
    /// Optional per-coach LLM sampling temperature override. `None` → use
    /// provider/server default.
    pub temperature: Option<f32>,
    /// Coach category — drives category-specific scope carve-outs injected
    /// into the system prompt (e.g. Nutrition coaches bypass the generic
    /// "food/meal finders" out-of-scope refusal for meal-planning questions).
    pub category: CoachCategory,
}

/// Database representation of a chat message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRecord {
    /// Unique message ID
    pub id: String,
    /// Conversation ID this message belongs to
    pub conversation_id: String,
    /// Role of the message sender (system, user, assistant)
    pub role: String,
    /// Message content
    pub content: String,
    /// Token count for this message (completion tokens for assistant messages)
    pub token_count: Option<i64>,
    /// Prompt tokens used to generate this message
    pub prompt_tokens: Option<i64>,
    /// LLM model used to generate this message
    pub model: Option<String>,
    /// Finish reason for assistant messages
    pub finish_reason: Option<String>,
    /// Validated structured payload extracted from an assistant reply (e.g. a
    /// `structured-workout` plan JSON). When present, clients render it as a
    /// rich card instead of the raw text. JSON-encoded string.
    pub structured_content: Option<String>,
    /// When the message was created (ISO 8601)
    pub created_at: String,
}

/// Summary of a conversation for listing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummary {
    /// Conversation ID
    pub id: String,
    /// Conversation title
    pub title: String,
    /// LLM model used
    pub model: String,
    /// Number of messages in the conversation
    pub message_count: i64,
    /// Total tokens used
    pub total_tokens: i64,
    /// Coach attached to the conversation, if any. Lets the listing group
    /// sessions by coach and label them with the coach's name.
    pub coach_id: Option<String>,
    /// When the conversation was created
    pub created_at: String,
    /// When the conversation was last updated
    pub updated_at: String,
}

/// Parameters for adding a message to a conversation
pub struct AddMessageParams<'a> {
    /// Tenant that owns the conversation — every message write is tenant-scoped
    /// so the `chat_conversations` ownership check matches the conversation CRUD path.
    pub tenant_id: TenantId,
    /// Conversation to add the message to
    pub conversation_id: &'a str,
    /// User who owns the conversation
    pub user_id: &'a str,
    /// Role of the message sender (`user`, `assistant`, `system`)
    pub role: &'a str,
    /// Message content
    pub content: &'a str,
    /// Completion token count (for assistant messages)
    pub token_count: Option<u32>,
    /// Reason the LLM stopped generating (e.g. `stop`, `length`)
    pub finish_reason: Option<&'a str>,
    /// Prompt token count for this interaction
    pub prompt_tokens: Option<u32>,
    /// LLM model identifier used for this message
    pub model: Option<&'a str>,
    /// Validated structured payload (e.g. a `structured-workout` plan JSON)
    /// extracted from the reply, persisted alongside the message text.
    pub structured_content: Option<&'a str>,
}

/// Database representation of a user's thumbs up/down feedback on a message.
///
/// One row per `(message_id, user_id)` — the rating toggles and the optional
/// `comment` captures a "what went wrong?" reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageFeedbackRecord {
    /// Unique feedback ID
    pub id: String,
    /// Message this feedback is attached to
    pub message_id: String,
    /// Conversation the message belongs to (denormalized for conversation-
    /// scoped loads and future per-coach analytics without a re-join)
    pub conversation_id: String,
    /// User who left the feedback
    pub user_id: String,
    /// Tenant the feedback belongs to (multi-tenant isolation)
    pub tenant_id: String,
    /// Rating value: `"up"` or `"down"`
    pub rating: String,
    /// Optional free-text reason captured on a thumbs-down
    pub comment: Option<String>,
    /// When the feedback was first left (ISO 8601)
    pub created_at: String,
    /// When the feedback was last changed (ISO 8601)
    pub updated_at: String,
}

/// Parameters for upserting a user's feedback on a message.
///
/// Upsert keyed on `(message_id, user_id)`: a repeat rating overwrites the
/// previous one and refreshes `comment`/`updated_at`. The write is gated on
/// the caller owning the conversation the message belongs to.
pub struct UpsertMessageFeedbackParams<'a> {
    /// Tenant that owns the conversation — every feedback write is tenant-scoped
    pub tenant_id: TenantId,
    /// Conversation the message belongs to (used in the ownership check)
    pub conversation_id: &'a str,
    /// Message being rated
    pub message_id: &'a str,
    /// User leaving the feedback (owner of the conversation)
    pub user_id: &'a str,
    /// Rating value: `"up"` or `"down"`
    pub rating: &'a str,
    /// Optional free-text reason (typically only set on a thumbs-down)
    pub comment: Option<&'a str>,
}
