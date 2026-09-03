// ABOUTME: Turn-level input and output types for the unified chat pipeline
// ABOUTME: TurnInput carries the turn's identifiers, content and pre-turn quota standing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Turn input and output types for [`super::run`].

use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_database::database::{ConversationRecord, MessageRecord};

use crate::envelope::QuotaState;
use crate::stages::coach_mention::MentionedCoach;

/// Result of creating a new conversation, including the validated model.
pub struct CreateConversationResult {
    /// The created conversation record.
    pub conversation: ConversationRecord,
}

/// Who authored the message a turn answers.
///
/// The pipeline's second stage writes the turn's prompt to `chat_messages` as
/// a `user` row before it answers, and every later turn reads that row back as
/// history. That is right for a message an athlete typed and wrong for one the
/// platform composed on their behalf: the row is attributed to them, appears
/// in their own thread as something they said, advances their read marker and
/// fans out to a group room they never posted in.
///
/// A proactive turn therefore declares itself here, and the prompt reaches the
/// model through the in-memory history instead of through a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnOrigin {
    /// The athlete sent this message. It is persisted as their turn.
    Athlete,
    /// The platform composed this prompt — a completed backfill re-asking the
    /// athlete's own earlier question, say. Nothing is written as the
    /// athlete's; only the reply is persisted, because the reply is real.
    Platform,
}

impl TurnOrigin {
    /// Whether this turn's prompt is written to the transcript as an athlete
    /// message.
    #[must_use]
    pub const fn persists_user_row(self) -> bool {
        matches!(self, Self::Athlete)
    }
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
    /// The reply's audience: `true` when the athlete is alone with the coach.
    ///
    /// `false` on a shared-room turn — a messaging group, or an in-app thread
    /// bound to a coaching group — where every member reads the reply. A
    /// shared room must never receive a user-scoped link, so the reconnect
    /// re-challenge renders its linkless copy there
    /// ([`crate::stages::auth_recovery`]).
    pub is_direct_message: bool,
    /// Raw user message content.
    pub content: String,
    /// Whether the athlete sent [`Self::content`], or the platform composed it.
    ///
    /// Drives whether the turn writes a `user` row at all; see [`TurnOrigin`].
    pub origin: TurnOrigin,
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
    /// The coach the athlete addressed by `@handle` in this message, when one
    /// resolved against their installed coaches.
    ///
    /// Governs this turn alone: the persona, tools, playbooks, followups,
    /// canary and attribution all follow it, while the conversation's stored
    /// `coach_id` is never written — the next plain message reverts to the
    /// conversation's own coach. Resolved once by
    /// [`crate::turn_service::execute`] so every surface routes the same way.
    ///
    /// Boxed because it carries the coach's whole runtime context: inline, it
    /// pushed the messaging ingress future past clippy's `large_futures`
    /// budget, and a turn that mentions nobody pays for none of it.
    pub mentioned_coach: Option<Box<MentionedCoach>>,
}

impl TurnInput {
    /// The coach this turn answers as: the mentioned coach when the athlete
    /// named one, otherwise the coach the conversation is bound to.
    ///
    /// Every per-coach read on the turn path goes through here, so a routed
    /// turn is consistently the mentioned coach's — prompt, memory scopes and
    /// attribution alike — rather than a persona swap on top of another
    /// coach's context.
    #[must_use]
    pub fn turn_coach_id<'a>(&'a self, conv: &'a ConversationRecord) -> Option<&'a str> {
        self.mentioned_coach
            .as_ref()
            .map_or(conv.coach_id.as_deref(), |mention| {
                Some(mention.coach_id.as_str())
            })
    }
}
