// ABOUTME: Repository trait definitions for the chat conversation persistence domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;

use pierre_core::models::AddMessageParams;
use pierre_core::models::UpsertMessageFeedbackParams;
use pierre_core::models::{ConversationParticipant, ConversationRecord, ConversationSummary};
use pierre_core::models::{MessageFeedbackRecord, MessageRecord, TenantId};

/// Chat conversation and message management repository.
///
/// Access is a membership question, not an ownership one. Every method that
/// takes a `user_id` alongside a `conversation_id` answers for a
/// *participant* — a row in `conversation_participants` for that user in
/// that tenant — so an athlete added to someone else's thread reads and
/// posts in it exactly like the owner. The owner is a participant row too.
/// The exceptions are named on the methods that keep owner semantics:
/// deleting a conversation, and the per-user counts and sweeps that size an
/// athlete's own footprint.
#[async_trait]
pub trait ChatRepository: Send + Sync {
    /// Create a new chat conversation.
    ///
    /// `coach_id` references a coach in the `coaches` table; the coach's
    /// `system_prompt` is the canonical persona source and is resolved at
    /// runtime via [`CoachesRepository::get_coach_runtime_context`].
    ///
    /// The creator is written as the conversation's `owner` participant in
    /// the same call, so the row is readable through the membership
    /// predicate from the moment it exists.
    async fn create_conversation(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        title: &str,
        model: &str,
        coach_id: Option<&str>,
        group_id: Option<&str>,
    ) -> AppResult<ConversationRecord>;
    /// Get a conversation by ID, when `user_id` is a participant in this tenant.
    ///
    /// This is the membership check every route and pipeline stage reuses:
    /// `None` means either the conversation does not exist or the caller is
    /// not in it, and the two are deliberately indistinguishable.
    async fn get_conversation(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Option<ConversationRecord>>;
    /// List the conversations `user_id` participates in, with pagination
    async fn list_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<ConversationSummary>>;
    /// Update conversation title (any participant)
    async fn update_conversation_title(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
        title: &str,
    ) -> AppResult<bool>;
    /// Stamp a conversation's channel of origin (`telegram`/`whatsapp`/…) into
    /// the durable `channel_type` column. Used by messaging-ingress so the
    /// client badge survives a later title rename. Tenant-scoped.
    async fn set_conversation_channel(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
        channel_type: &str,
    ) -> AppResult<bool>;
    /// Delete a conversation and its messages.
    ///
    /// Owner-only: `user_id` must be the conversation's `user_id` column, not
    /// merely a participant. Returns `false` for a participant who is not the
    /// owner, so the route can tell "not yours to delete" from "not found".
    async fn delete_conversation(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool>;
    /// Add a message to a conversation (verifies the user is a participant)
    async fn add_message(&self, params: &AddMessageParams<'_>) -> AppResult<MessageRecord>;
    /// Get all messages for a conversation (verifies the user is a participant)
    async fn get_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<MessageRecord>>;
    /// Get recent messages for a conversation (verifies the user is a participant)
    async fn get_recent_messages(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
        limit: i64,
    ) -> AppResult<Vec<MessageRecord>>;
    /// Get message count for a conversation (verifies the user is a participant)
    async fn get_message_count(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<i64>;

    /// Upsert the caller's thumbs up/down feedback on a single message.
    ///
    /// Keyed on `(message_id, user_id)`: a repeat rating overwrites the prior
    /// one and refreshes the comment + `updated_at`. The write only lands when
    /// the message belongs to a conversation the caller participates in, in
    /// this tenant — otherwise it returns `NotFound`.
    async fn upsert_message_feedback(
        &self,
        params: &UpsertMessageFeedbackParams<'_>,
    ) -> AppResult<MessageFeedbackRecord>;

    /// Remove the caller's feedback on a message (thumbs toggle-off).
    /// Tenant-scoped; returns `false` when no feedback row existed.
    async fn delete_message_feedback(
        &self,
        message_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool>;

    /// Load all of the caller's feedback rows for a conversation, so the
    /// client can re-render thumbs state after a reload. Tenant-scoped.
    async fn get_conversation_feedback(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<MessageFeedbackRecord>>;
    /// Count the conversations a user *owns* in a tenant. Owner semantics on
    /// purpose: this sizes the `max_active_conversations` quota, and a thread
    /// someone else opened must not count against the athlete added to it.
    async fn count_conversations(&self, user_id: &str, tenant_id: TenantId) -> AppResult<i64>;
    /// Delete all conversations a user *owns* (account cleanup). Owner
    /// semantics: the athlete's own threads go; their membership in other
    /// people's threads is not theirs to destroy.
    async fn delete_all_user_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<i64>;

    /// Get recently updated conversations across all tenants (admin view)
    ///
    /// Returns the last `limit` conversations ordered by `updated_at` descending.
    /// Includes the associated `user_id` for display. Used by the admin activity dashboard.
    async fn get_recent_conversations_admin(
        &self,
        limit: i64,
    ) -> AppResult<Vec<ConversationRecord>>;

    /// Count conversations updated since a given timestamp (admin view, cross-tenant)
    async fn count_active_conversations_since(&self, since: &str) -> AppResult<i64>;

    /// Attach a coach session id to an existing conversation row (Tier 4
    /// cross-channel continuity). Tenant-scoped; returns `false` if the
    /// conversation does not exist.
    async fn set_conversation_session_id(
        &self,
        conversation_id: &str,
        session_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool>;

    /// Set (or clear) the guided pillar-onboarding state JSON on a conversation.
    ///
    /// `/pillars` writes the active state to enter onboarding mode; the flow
    /// clears it (`None`) once all pillars are covered. Tenant-scoped; returns
    /// `false` if the conversation does not exist.
    async fn set_conversation_onboarding_state(
        &self,
        conversation_id: &str,
        onboarding_state: Option<&str>,
        tenant_id: TenantId,
    ) -> AppResult<bool>;

    /// Set (or clear) the guided-interview state only while the column still
    /// holds `expected` — a compare-and-set on the conversation row.
    ///
    /// An LLM turn reads this column when it starts and writes it back tens of
    /// seconds later, while slash commands are handled synchronously in the
    /// webhook path outside the dispatch lock. An athlete who types
    /// `/calibrate` mid-turn therefore has a fresh interview written under the
    /// running turn, and a blind write-back would replace it with the snapshot
    /// that turn loaded — reverting the interview they just started. Passing
    /// the turn-start value as `expected` makes that write a no-op instead.
    ///
    /// `expected` is matched NULL-safely, so "the conversation carried no
    /// state" is a value like any other. Tenant-scoped; returns `true` when the
    /// row was updated and `false` when it was not — either because a newer
    /// state owns the column or because the conversation does not exist.
    async fn compare_and_set_conversation_onboarding_state(
        &self,
        conversation_id: &str,
        expected: Option<&str>,
        onboarding_state: Option<&str>,
        tenant_id: TenantId,
    ) -> AppResult<bool>;

    /// The raw `onboarding_state` JSON of this user's conversations that carry
    /// one, newest-updated first, capped at `limit`.
    ///
    /// Keyed on the athlete rather than one conversation, because a `tools/call`
    /// arriving on the `/mcp` endpoint has no conversation in scope and the
    /// guided-flow write guard still has to answer "is an interview running for
    /// this athlete right now?". Returns the columns verbatim — whether a stored
    /// state is *active* or a finished marker is the caller's read
    /// (`OnboardingState::from_column`), so the two never disagree.
    /// Tenant-scoped.
    async fn list_user_onboarding_states(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        limit: i64,
    ) -> AppResult<Vec<String>>;

    /// Attach a coaching group id to an existing conversation row.
    ///
    /// Used by the messaging-ingress auto-bind path to retrofit
    /// `chat_conversations.group_id` onto a conversation that pre-dates
    /// the channel/group binding (legacy sessions, or a freshly forged
    /// self-heal conversation). Tenant-scoped; returns `false` if the
    /// conversation does not exist.
    async fn set_conversation_group_id(
        &self,
        conversation_id: &str,
        group_id: Option<&str>,
        tenant_id: TenantId,
    ) -> AppResult<bool>;

    /// Point an existing conversation at a different coach.
    ///
    /// A messaging channel holds one long-lived conversation per athlete, so
    /// the coach they pick has to reach the thread they are already in.
    /// Rebinding the row does that while keeping the history; forging a fresh
    /// conversation is what `/reset` does, and losing the thread was the price
    /// of every coach change before this existed.
    ///
    /// Tenant-scoped; returns `false` if the conversation does not exist.
    async fn set_conversation_coach_id(
        &self,
        conversation_id: &str,
        coach_id: Option<&str>,
        tenant_id: TenantId,
    ) -> AppResult<bool>;

    /// Add `user_id` to a conversation as a `member`, recorded as added by
    /// `added_by`. Idempotent: re-adding an existing participant returns the
    /// row that already exists (the owner keeps the `owner` role).
    ///
    /// The conversation must exist in `tenant_id`; otherwise `NotFound`.
    /// Whether `user_id` belongs to that tenant is the caller's check —
    /// the route refuses a cross-tenant add before reaching here.
    async fn add_participant(
        &self,
        conversation_id: &str,
        tenant_id: TenantId,
        user_id: &str,
        added_by: &str,
    ) -> AppResult<ConversationParticipant>;

    /// Remove a `member` from a conversation. Returns `false` when there was
    /// no such member row — including when `user_id` is the owner, whose row
    /// this never touches. Tenant-scoped.
    async fn remove_participant(
        &self,
        conversation_id: &str,
        tenant_id: TenantId,
        user_id: &str,
    ) -> AppResult<bool>;

    /// Every participant of a conversation, owner first, then members in the
    /// order they were added. Tenant-scoped; an unknown conversation yields
    /// an empty list, which is why callers gate on `get_conversation` first.
    async fn list_participants(
        &self,
        conversation_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<Vec<ConversationParticipant>>;
}
