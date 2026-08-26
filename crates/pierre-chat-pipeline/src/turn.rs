// ABOUTME: Turn-level input and output types for the unified chat pipeline
// ABOUTME: TurnInput carries the turn's identifiers, content and pre-turn quota standing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Turn input and output types for [`super::run`].

use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_database::database::{ConversationRecord, MessageRecord};

use crate::envelope::QuotaState;

/// Result of creating a new conversation, including the validated model.
pub struct CreateConversationResult {
    /// The created conversation record.
    pub conversation: ConversationRecord,
}

/// Result of persisting a user message and resolving the parent conversation.
pub struct UserMessageResult {
    /// The persisted user message.
    pub message: MessageRecord,
    /// The conversation record (for model / `coach_id` access during dispatch).
    pub conversation: ConversationRecord,
}

/// Input to a single pipeline turn.
///
/// `conversation_tenant_id` is used for conversation/message DB lookups.
/// `tool_tenant_id` is used for tool execution (OAuth, activities, etc.).
/// These may differ when a messaging user belongs to a different tenant
/// than the bot that owns the channel webhook.
#[derive(Debug, Clone)]
pub struct TurnInput {
    /// Identifier of the conversation the turn is appending to.
    pub conversation_id: String,
    /// User UUID as string.
    pub user_id: String,
    /// Tenant used for conversation and message DB lookups.
    pub conversation_tenant_id: TenantId,
    /// Tenant used for tool execution (OAuth credentials, provider APIs).
    pub tool_tenant_id: TenantId,
    /// Raw user message content.
    pub content: String,
    /// Conversation-turn correlation identifier generated at the inbound
    /// boundary (web chat handler, messaging ingress, CLI entry). Threaded
    /// through every downstream LLM call and the persisted LLM usage row so
    /// per-turn observability can attribute cost/latency/tools to the
    /// originating utterance.
    pub turn_id: ConversationTurnId,
    /// Pre-rendered ambient-context block appended to the system prompt.
    ///
    /// Group messaging turns carry the room's recent speaker-labeled
    /// transcript here (built by the messaging ingress from the shared
    /// `group_transcript_entries` read model, consent-gated per viewer) so
    /// the coach can answer "what do you think of the plan above?" — each
    /// member's `chat_messages` history holds only their own exchanges.
    /// `None` for web chat and DM turns.
    pub ambient_context: Option<String>,
    /// Where the athlete stood against their usage caps when the turn was
    /// admitted.
    ///
    /// Measured by [`crate::turn_service::execute`]'s pre-turn check, which is
    /// also what refuses a hard breach — so the warning the athlete is shown
    /// and the counters that admitted the turn are the same measurement. Rides
    /// the envelope out as a [`crate::ReplyBlock::Notice`].
    pub quota: QuotaState,
}
