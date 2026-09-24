// ABOUTME: Repository trait definitions for the chat conversation persistence domain
// ABOUTME: The statements written once with $n placeholders; chat_backend.rs emits the implementation per backend
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};

use pierre_core::models::AddMessageParams;
use pierre_core::models::UpsertMessageFeedbackParams;
use pierre_core::models::{ConversationPage, ConversationParticipant, ConversationRecord};
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
    /// `agent_id` references an agent in the `agents` table; the agent's
    /// `system_prompt` is the canonical persona source and is resolved at
    /// runtime via [`AgentsRepository::get_agent_runtime_context`].
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
        agent_id: Option<&str>,
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
    /// One page of the conversations `user_id` participates in — whatever
    /// surface opened them — newest activity first, with the participant's
    /// total alongside so a client pages against the real count.
    ///
    /// Each row carries what a list row shows: the agent's title and
    /// `@handle`, the group's name, the newest `user`/`assistant` row and the
    /// count of such rows written after the caller's read marker. Tool rows
    /// count nowhere. `limit`/`offset` are applied as given; the route clamps
    /// them.
    async fn list_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
        limit: i64,
        offset: i64,
    ) -> AppResult<ConversationPage>;
    /// Count every conversation `user_id` participates in, in this tenant —
    /// the `total` of [`Self::list_conversations`]. Membership semantics, so
    /// it must never size a quota: that is [`Self::count_conversations`].
    async fn count_participating_conversations(
        &self,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<i64>;
    /// Advance `user_id`'s read marker on a conversation they participate in.
    ///
    /// `up_to_message_id` names the row the athlete has seen; `None` means the
    /// newest `user`/`assistant` row. The marker is monotonic — it never moves
    /// backwards, so a stale client re-marking an older row cannot resurrect
    /// unread rows — and it is the participant's own: another member's marker
    /// is untouched. Returns `false` when the caller is not a participant or
    /// the named message is not in this conversation, both of which the route
    /// answers with 404; a conversation with no rows yet marks nothing and
    /// still returns `true`.
    async fn mark_conversation_read(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
        up_to_message_id: Option<&str>,
    ) -> AppResult<bool>;
    /// Clear `user_id`'s read marker — "mark unread": every `user`/`assistant`
    /// row counts as unread again. Returns `false` when the caller is not a
    /// participant.
    async fn clear_conversation_read_marker(
        &self,
        conversation_id: &str,
        user_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool>;
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

    /// Attach an agent session id to an existing conversation row (Tier 4
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

    /// Point an existing conversation at a different agent.
    ///
    /// A messaging channel holds one long-lived conversation per athlete, so
    /// the agent they pick has to reach the thread they are already in.
    /// Rebinding the row does that while keeping the history; forging a fresh
    /// conversation is what `/reset` does, and losing the thread was the price
    /// of every agent change before this existed.
    ///
    /// Tenant-scoped; returns `false` if the conversation does not exist.
    async fn set_conversation_agent_id(
        &self,
        conversation_id: &str,
        agent_id: Option<&str>,
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

    /// Whether `agent_id` has already introduced itself in `thread_id`: the
    /// coaching group id of a shared room, else the conversation id.
    /// Tenant-scoped.
    async fn has_agent_introduction(
        &self,
        thread_id: &str,
        agent_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<bool>;

    /// Record that `agent_id` has introduced itself in `thread_id`, once a
    /// reply that named it reached the thread. Idempotent.
    async fn record_agent_introduction(
        &self,
        thread_id: &str,
        agent_id: &str,
        tenant_id: TenantId,
    ) -> AppResult<()>;
}

/// How many leading characters of the newest row travel with a list row.
///
/// The route shapes the preview (marker strip, whitespace collapse, 120
/// characters); this bound keeps a thread whose last reply is a full training
/// plan from shipping the whole plan to draw one line. An `i32`, because
/// Postgres' `SUBSTR` takes an `int4` and `SQLite` takes either.
pub(crate) const CONTENT_HEAD_CHARS: i32 = 512;

/// The thirteen columns every conversation read returns, in the order
/// `impl_chat_repository!`'s `conversation_from_row` reads them, aliased
/// on `c` so the same list serves a joined read and a plain one.
macro_rules! conversation_columns {
    () => {
        "c.id, c.user_id, c.tenant_id, c.title, c.model, c.agent_id, c.session_id, \
         c.total_tokens, c.created_at, c.updated_at, c.group_id, c.channel_type, \
         c.onboarding_state"
    };
}

/// The ten columns every message read returns.
macro_rules! message_columns {
    () => {
        "m.id, m.conversation_id, m.role, m.content, m.token_count, m.prompt_tokens, \
         m.model, m.finish_reason, m.content_blocks, m.created_at"
    };
}

/// The nine columns every feedback read returns.
macro_rules! feedback_columns {
    () => {
        "id, message_id, conversation_id, user_id, tenant_id, rating, comment, \
         created_at, updated_at"
    };
}

/// The membership predicate every participant-scoped read shares: the
/// caller has a participant row on the conversation in the conversation's
/// tenant, and the conversation is either in the caller's tenant or a
/// group room. `$1` is the conversation, `$2` the caller, `$3` the tenant.
///
/// A GROUP row is authorized by the participant row rather than by the
/// caller's tenant. Every other query in this crate gates on the CALLER's
/// tenant, and a 1:1 thread still does here. A channel group cannot: its
/// session and its conversation are stored under the channel/bot tenant on
/// purpose, because a room's members may span tenants and its
/// `coaching_group` must resolve to ONE row for all of them
/// (`messaging_ingress/session.rs`, shipped 543c4c32f). Measured on deployed
/// dev 2026-09-03: all 14 group conversations sat in the bot tenant and five
/// of their six owners were not members of it, so every one of those rooms
/// was invisible in web and mobile while working in Telegram. For a group
/// row the participant row is the stronger statement anyway — it names this
/// person on this thread, where a shared tenant only says they are in the
/// same building. `p.tenant_id = c.tenant_id` keeps a stray cross-tenant
/// membership from granting anything, and a non-participant still matches
/// nothing at all. Writes are unchanged and still require the caller's tenant.
macro_rules! participant_scope {
    () => {
        "p.user_id = $2 AND p.tenant_id = c.tenant_id \
         AND (c.tenant_id = $3 OR c.group_id IS NOT NULL)"
    };
}

/// Insert the conversation row. `$n` placeholders throughout this module:
/// sqlx accepts them on `SQLite` as well as Postgres. Ids the caller holds
/// as text bind through the backend's codec (`user_id` and `group_id` are
/// `uuid` columns on Postgres, TEXT on `SQLite`); timestamps bind as
/// `DateTime<Utc>` on both, which sqlx-sqlite writes as the RFC 3339 text
/// the columns hold there.
pub(crate) const CREATE_CONVERSATION_SQL: &str = r"
    INSERT INTO chat_conversations (id, user_id, tenant_id, title, model, agent_id, group_id, total_tokens, created_at, updated_at)
    VALUES ($1, $2, $3, $4, $5, $6, $7, 0, $8, $8)";

/// The owner's participant row, written in the same transaction as the
/// conversation: every read path answers through the membership predicate,
/// so a conversation without its owner row would be invisible to the
/// athlete who just opened it.
pub(crate) const CREATE_OWNER_PARTICIPANT_SQL: &str = r"
    INSERT INTO conversation_participants (conversation_id, user_id, tenant_id, role, added_by, added_at)
    VALUES ($1, $2, $3, $4, $2, $5)";

/// One conversation, for a participant.
pub(crate) const GET_CONVERSATION_SQL: &str = concat!(
    "SELECT ",
    conversation_columns!(),
    " FROM chat_conversations c \
     WHERE c.id = $1 \
       AND (c.tenant_id = $3 OR c.group_id IS NOT NULL) \
       AND EXISTS ( \
         SELECT 1 FROM conversation_participants p \
         WHERE p.conversation_id = c.id AND p.user_id = $2 \
           AND p.tenant_id = c.tenant_id \
       )"
);

/// One page of a participant's conversations plus the facts each row shows.
///
/// The statement asks the row-level questions inline — count of turns, count
/// of unread turns, and the newest turn — as correlated scalar subqueries over
/// `chat_messages`, each served by the `(conversation_id, created_at)` index.
/// A `GROUP BY` over a `LEFT JOIN chat_messages` counted tool rows and could
/// not say which row was newest; three scoped subqueries can.
///
/// `p` is the caller's own participant row, which is where their read marker
/// lives: "unread" is a question about one participant, so it is answered
/// from that row and never from the conversation.
pub(crate) const LIST_CONVERSATIONS_SQL: &str = r"
    SELECT c.id, c.title, c.model, c.total_tokens, c.agent_id, c.channel_type,
           c.created_at, c.updated_at, c.group_id,
           g.name AS group_name, co.slug AS agent_handle, co.title AS agent_title,
           (SELECT COUNT(*) FROM chat_messages m
             WHERE m.conversation_id = c.id AND m.role IN ('user', 'assistant')) AS message_count,
           (SELECT COUNT(*) FROM chat_messages m
             WHERE m.conversation_id = c.id AND m.role IN ('user', 'assistant')
               AND (p.last_read_at IS NULL OR m.created_at > p.last_read_at)) AS unread_count,
           (SELECT SUBSTR(m.content, 1, $5) FROM chat_messages m
             WHERE m.conversation_id = c.id AND m.role IN ('user', 'assistant')
             ORDER BY m.created_at DESC, m.id DESC LIMIT 1) AS last_content_head,
           (SELECT m.role FROM chat_messages m
             WHERE m.conversation_id = c.id AND m.role IN ('user', 'assistant')
             ORDER BY m.created_at DESC, m.id DESC LIMIT 1) AS last_role,
           (SELECT m.created_at FROM chat_messages m
             WHERE m.conversation_id = c.id AND m.role IN ('user', 'assistant')
             ORDER BY m.created_at DESC, m.id DESC LIMIT 1) AS last_created_at
    FROM chat_conversations c
    JOIN conversation_participants p ON p.conversation_id = c.id
    LEFT JOIN coaching_groups g ON g.id = c.group_id
    LEFT JOIN agents co ON co.id = c.agent_id
    WHERE p.user_id = $1
      AND p.tenant_id = c.tenant_id
      AND (c.tenant_id = $2 OR c.group_id IS NOT NULL)
    ORDER BY c.updated_at DESC, c.id DESC
    LIMIT $3 OFFSET $4";

/// The participant's total — the same membership predicate as the page.
pub(crate) const COUNT_PARTICIPATING_SQL: &str = r"
    SELECT COUNT(*)
    FROM chat_conversations c
    JOIN conversation_participants p ON p.conversation_id = c.id
    WHERE p.user_id = $1
      AND p.tenant_id = c.tenant_id
      AND (c.tenant_id = $2 OR c.group_id IS NOT NULL)";

/// The instant a read marker should advance to, gated on membership.
///
/// Answers for a participant only — a stranger gets no row at all, which the
/// caller reports as `false`. `$4` is the message the athlete has seen; when
/// it is absent the newest `user`/`assistant` row is the target, and an empty
/// conversation yields a `NULL` target with the membership row still present.
pub(crate) const READ_TARGET_SQL: &str = concat!(
    "SELECT ( \
        SELECT MAX(m.created_at) FROM chat_messages m \
        WHERE m.conversation_id = p.conversation_id \
          AND (($4 IS NULL AND m.role IN ('user', 'assistant')) OR m.id = $4) \
     ) AS target \
     FROM conversation_participants p \
     JOIN chat_conversations c ON c.id = p.conversation_id \
     WHERE p.conversation_id = $1 AND ",
    participant_scope!()
);

/// Advance the marker, never backwards: a stale client re-marking an older
/// row leaves a newer marker where it is.
pub(crate) const ADVANCE_READ_MARKER_SQL: &str = r"
    UPDATE conversation_participants
    SET last_read_at = $4
    WHERE conversation_id = $1 AND user_id = $2
      AND EXISTS (
        SELECT 1 FROM chat_conversations c
        WHERE c.id = conversation_participants.conversation_id
          AND conversation_participants.tenant_id = c.tenant_id
          AND (c.tenant_id = $3 OR c.group_id IS NOT NULL)
      )
      AND (last_read_at IS NULL OR last_read_at < $4)";

/// Clear the marker (mark unread).
pub(crate) const CLEAR_READ_MARKER_SQL: &str = r"
    UPDATE conversation_participants
    SET last_read_at = NULL
    WHERE conversation_id = $1 AND user_id = $2
      AND EXISTS (
        SELECT 1 FROM chat_conversations c
        WHERE c.id = conversation_participants.conversation_id
          AND conversation_participants.tenant_id = c.tenant_id
          AND (c.tenant_id = $3 OR c.group_id IS NOT NULL)
      )";

/// Rename a conversation, for a participant in the caller's tenant.
pub(crate) const UPDATE_TITLE_SQL: &str = r"
    UPDATE chat_conversations
    SET title = $1, updated_at = $2
    WHERE id = $3 AND tenant_id = $5
      AND EXISTS (
        SELECT 1 FROM conversation_participants p
        WHERE p.conversation_id = chat_conversations.id AND p.user_id = $4 AND p.tenant_id = $5
      )";

/// Stamp a conversation's channel of origin, for a participant.
pub(crate) const SET_CHANNEL_SQL: &str = r"
    UPDATE chat_conversations
    SET channel_type = $1
    WHERE id = $2 AND tenant_id = $4
      AND EXISTS (
        SELECT 1 FROM conversation_participants p
        WHERE p.conversation_id = chat_conversations.id AND p.user_id = $3 AND p.tenant_id = $4
      )";

/// Delete a conversation; owner-only, the messages cascade.
pub(crate) const DELETE_CONVERSATION_SQL: &str = r"
    DELETE FROM chat_conversations
    WHERE id = $1 AND user_id = $2 AND tenant_id = $3";

/// Insert a message only when the caller is a participant of the
/// conversation in this tenant.
pub(crate) const ADD_MESSAGE_SQL: &str = r"
    INSERT INTO chat_messages (id, conversation_id, role, content, token_count, finish_reason, created_at, prompt_tokens, model, content_blocks)
    SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $12
    WHERE EXISTS (
        SELECT 1 FROM chat_conversations c
        JOIN conversation_participants p ON p.conversation_id = c.id
        WHERE c.id = $2 AND c.tenant_id = $11 AND p.user_id = $10 AND p.tenant_id = $11
    )";

/// Touch the conversation and add the message's tokens to its total.
pub(crate) const BUMP_CONVERSATION_TOKENS_SQL: &str = r"
    UPDATE chat_conversations
    SET updated_at = $1, total_tokens = total_tokens + $2
    WHERE id = $3 AND tenant_id = $4";

/// Touch the conversation.
pub(crate) const TOUCH_CONVERSATION_SQL: &str = r"
    UPDATE chat_conversations
    SET updated_at = $1
    WHERE id = $2 AND tenant_id = $3";

/// Every message of a conversation, oldest first, for a participant.
pub(crate) const GET_MESSAGES_SQL: &str = concat!(
    "SELECT ",
    message_columns!(),
    " FROM chat_messages m \
     JOIN chat_conversations c ON m.conversation_id = c.id \
     JOIN conversation_participants p ON p.conversation_id = c.id \
     WHERE m.conversation_id = $1 AND ",
    participant_scope!(),
    " ORDER BY m.created_at ASC"
);

/// The newest `$4` messages of a conversation, newest first, for a
/// participant; the caller reverses them.
pub(crate) const GET_RECENT_MESSAGES_SQL: &str = concat!(
    "SELECT ",
    message_columns!(),
    " FROM chat_messages m \
     JOIN chat_conversations c ON m.conversation_id = c.id \
     JOIN conversation_participants p ON p.conversation_id = c.id \
     WHERE m.conversation_id = $1 AND ",
    participant_scope!(),
    " ORDER BY m.created_at DESC, m.id DESC \
     LIMIT $4"
);

/// How many messages a conversation holds, for a participant.
pub(crate) const MESSAGE_COUNT_SQL: &str = concat!(
    "SELECT COUNT(*) \
     FROM chat_messages m \
     JOIN chat_conversations c ON m.conversation_id = c.id \
     JOIN conversation_participants p ON p.conversation_id = c.id \
     WHERE m.conversation_id = $1 AND ",
    participant_scope!()
);

/// Insert keyed on (`message_id`, `user_id`); on a repeat rating, overwrite the
/// rating + comment and bump `updated_at`. The WHERE EXISTS gate lands the row
/// only when the message belongs to a conversation the caller participates
/// in, in this tenant — a forged `message_id` never inserts.
pub(crate) const UPSERT_FEEDBACK_SQL: &str = r"
    INSERT INTO chat_message_feedback
        (id, message_id, conversation_id, user_id, tenant_id, rating, comment, created_at, updated_at)
    SELECT $1, $2, $3, $4, $5, $6, $7, $8, $8
    WHERE EXISTS (
        SELECT 1 FROM chat_messages m
        JOIN chat_conversations c ON m.conversation_id = c.id
        JOIN conversation_participants p ON p.conversation_id = c.id
        WHERE m.id = $2 AND m.conversation_id = $3 AND p.user_id = $4 AND p.tenant_id = $5 AND c.tenant_id = $5
    )
    ON CONFLICT (message_id, user_id) DO UPDATE SET
        rating = EXCLUDED.rating,
        comment = EXCLUDED.comment,
        updated_at = EXCLUDED.updated_at";

/// The caller's feedback on one message: the canonical row, read back after
/// an upsert because on conflict the stored id and `created_at` stay the
/// original values.
pub(crate) const GET_FEEDBACK_SQL: &str = concat!(
    "SELECT ",
    feedback_columns!(),
    " FROM chat_message_feedback \
     WHERE message_id = $1 AND user_id = $2 AND tenant_id = $3"
);

/// Delete the caller's feedback on a message (thumbs toggle-off).
pub(crate) const DELETE_FEEDBACK_SQL: &str = r"
    DELETE FROM chat_message_feedback
    WHERE message_id = $1 AND user_id = $2 AND tenant_id = $3";

/// All of the caller's feedback rows for a conversation.
pub(crate) const CONVERSATION_FEEDBACK_SQL: &str = concat!(
    "SELECT ",
    feedback_columns!(),
    " FROM chat_message_feedback \
     WHERE conversation_id = $1 AND user_id = $2 AND tenant_id = $3"
);

/// A user's own conversations in a tenant (owner semantics).
pub(crate) const COUNT_CONVERSATIONS_SQL: &str = r"
    SELECT COUNT(*)
    FROM chat_conversations
    WHERE user_id = $1 AND tenant_id = $2";

/// Delete a user's own conversations in a tenant (account cleanup).
pub(crate) const DELETE_USER_CONVERSATIONS_SQL: &str = r"
    DELETE FROM chat_conversations
    WHERE user_id = $1 AND tenant_id = $2";

/// The newest conversations across every tenant, for the operator console.
pub(crate) const RECENT_CONVERSATIONS_ADMIN_SQL: &str = concat!(
    "SELECT ",
    conversation_columns!(),
    " FROM chat_conversations c \
     ORDER BY c.updated_at DESC \
     LIMIT $1"
);

/// Conversations touched since an instant, across every tenant.
pub(crate) const COUNT_ACTIVE_SINCE_SQL: &str =
    "SELECT COUNT(*) FROM chat_conversations WHERE updated_at >= $1";

/// Bind a conversation to its messaging session.
pub(crate) const SET_SESSION_ID_SQL: &str = r"
    UPDATE chat_conversations
    SET session_id = $1
    WHERE id = $2 AND tenant_id = $3";

/// Replace the onboarding state unconditionally.
pub(crate) const SET_ONBOARDING_STATE_SQL: &str = r"
    UPDATE chat_conversations
    SET onboarding_state = $1
    WHERE id = $2 AND tenant_id = $3";

/// Replace the onboarding state only when it still holds `$4`. `IS NOT
/// DISTINCT FROM` is the NULL-safe equality both engines share: a
/// conversation that carried no state matches only an absent `expected`,
/// never any stored JSON.
pub(crate) const CAS_ONBOARDING_STATE_SQL: &str = r"
    UPDATE chat_conversations
    SET onboarding_state = $1
    WHERE id = $2 AND tenant_id = $3
      AND onboarding_state IS NOT DISTINCT FROM $4";

/// A user's onboarding states, newest activity first.
pub(crate) const LIST_ONBOARDING_STATES_SQL: &str = r"
    SELECT onboarding_state
    FROM chat_conversations
    WHERE user_id = $1 AND tenant_id = $2 AND onboarding_state IS NOT NULL
    ORDER BY updated_at DESC
    LIMIT $3";

/// Bind a conversation to a coaching group, or unbind it.
pub(crate) const SET_GROUP_ID_SQL: &str = r"
    UPDATE chat_conversations
    SET group_id = $1
    WHERE id = $2 AND tenant_id = $3";

/// Bind a conversation to an agent, or unbind it. `agent_id` is TEXT on
/// both backends — agent ids are slugs, not uuids — so unlike `group_id`
/// there is nothing to parse.
pub(crate) const SET_AGENT_ID_SQL: &str = r"
    UPDATE chat_conversations
    SET agent_id = $1
    WHERE id = $2 AND tenant_id = $3";

/// Whether an agent has introduced itself in a thread. `thread_id` is text on
/// both backends: a conversation id, or a group id already rendered as text.
pub(crate) const HAS_AGENT_INTRODUCTION_SQL: &str = r"
    SELECT 1 FROM agent_introductions
    WHERE tenant_id = $1 AND thread_id = $2 AND agent_id = $3";

/// Record an introduction; a second write for the same thread and agent — two
/// turns racing in one room — keeps the first.
pub(crate) const RECORD_AGENT_INTRODUCTION_SQL: &str = r"
    INSERT INTO agent_introductions (tenant_id, thread_id, agent_id, created_at)
    VALUES ($1, $2, $3, $4)
    ON CONFLICT DO NOTHING";

/// Add a member, idempotently. `ON CONFLICT DO NOTHING` keeps an existing
/// row (the owner's included) untouched; the WHERE EXISTS gate refuses a
/// conversation outside this tenant instead of writing a dangling
/// membership.
pub(crate) const ADD_PARTICIPANT_SQL: &str = r"
    INSERT INTO conversation_participants
        (conversation_id, user_id, tenant_id, role, added_by, added_at)
    SELECT $1, $2, $3, $4, $5, $6
    WHERE EXISTS (
        SELECT 1 FROM chat_conversations WHERE id = $1 AND tenant_id = $3
    )
    ON CONFLICT (conversation_id, user_id) DO NOTHING";

/// One membership row.
pub(crate) const GET_PARTICIPANT_SQL: &str = r"
    SELECT conversation_id, user_id, tenant_id, role, added_by, added_at
    FROM conversation_participants
    WHERE conversation_id = $1 AND user_id = $2 AND tenant_id = $3";

/// Remove a member; the owner's row is never removed.
pub(crate) const REMOVE_PARTICIPANT_SQL: &str = r"
    DELETE FROM conversation_participants
    WHERE conversation_id = $1 AND user_id = $2 AND tenant_id = $3 AND role = $4";

/// Every participant of a conversation, owner first.
pub(crate) const LIST_PARTICIPANTS_SQL: &str = r"
    SELECT conversation_id, user_id, tenant_id, role, added_by, added_at
    FROM conversation_participants
    WHERE conversation_id = $1 AND tenant_id = $2
    ORDER BY CASE role WHEN 'owner' THEN 0 ELSE 1 END, added_at ASC, user_id ASC";

/// The error one column read raises.
pub(crate) fn chat_column_error(column: &str, e: &sqlx::Error) -> AppError {
    AppError::database(format!("chat column {column}: {e}"))
}

/// Read a timestamp column of either backend in the RFC 3339 text the wire
/// DTOs carry: a TIMESTAMPTZ on Postgres, RFC 3339 text on `SQLite` that
/// sqlx parses and this renders again — the same text, for a value this
/// repository wrote.
///
/// # Errors
/// Returns a database error naming the column when it cannot be decoded.
pub(crate) fn stamp_column<R>(row: &R, column: &str) -> AppResult<String>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let at: DateTime<Utc> = row
        .try_get(column)
        .map_err(|e| chat_column_error(column, &e))?;
    Ok(at.to_rfc3339())
}

/// Parse the RFC 3339 instant a caller hands the repository as text.
///
/// # Errors
/// Returns an invalid-input error when the text is not an RFC 3339 instant.
pub(crate) fn instant_from_rfc3339(since: &str) -> AppResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(since)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| AppError::invalid_input(format!("Invalid RFC 3339 instant '{since}': {e}")))
}
