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
}

/// Runtime context for a coach attached to a conversation.
///
/// Consolidates the handful of coach fields the chat pipeline needs on every
/// turn (system prompt, startup context, tool-iteration override) into a
/// single tenant-scoped lookup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoachRuntimeContext {
    /// The coach's system prompt text (canonical source)
    pub system_prompt: String,
    /// Optional startup query the coach wants injected on the first turn
    pub startup_query: Option<String>,
    /// Optional JSON-encoded data requirements for deterministic pre-fetch
    pub data_requirements: Option<String>,
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
    /// When the conversation was created
    pub created_at: String,
    /// When the conversation was last updated
    pub updated_at: String,
}

/// Parameters for adding a message to a conversation
pub struct AddMessageParams<'a> {
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
}
