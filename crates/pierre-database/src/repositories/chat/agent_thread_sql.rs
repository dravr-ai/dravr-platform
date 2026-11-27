// ABOUTME: Statements over a thread's agent and introductions, the athlete's recent words, and assistant replies
// ABOUTME: Run by impl_chat_repository!; re-exported from the chat repository module

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

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

/// The athlete's own `user` rows at or after `$3`. `since` binds as a
/// timestamp, so the comparison is the one the column was written with on
/// either backend.
pub(crate) const USER_WROTE_SINCE_SQL: &str = r"
    SELECT COUNT(*) FROM chat_messages m
    INNER JOIN chat_conversations c ON c.id = m.conversation_id
    WHERE c.tenant_id = $1 AND c.user_id = $2 AND m.role = 'user' AND m.created_at >= $3";

/// The backfill walk over a tenant's assistant replies, oldest first.
///
/// The cursor is resolved in the same statement: `mark` is the cursor
/// message's `(created_at, id)`, empty when `$3` is NULL or names no stored
/// message, and every row after it in `(created_at, id)` order qualifies.
/// Both bounds bind as timestamps, so the comparison is the one the column
/// was written with on either backend.
pub(crate) const ASSISTANT_MESSAGES_AFTER_SQL: &str = r"
    WITH mark AS (SELECT created_at, id FROM chat_messages WHERE id = $3)
    SELECT m.id AS message_id,
           m.conversation_id AS conversation_id,
           c.user_id AS user_id,
           c.agent_id AS agent_id,
           m.content AS content,
           m.first_party_only AS first_party_only,
           m.ai_consent_sources AS ai_consent_sources
    FROM chat_messages m
    INNER JOIN chat_conversations c ON c.id = m.conversation_id
    WHERE c.tenant_id = $1
      AND m.role = 'assistant'
      AND ($2 IS NULL OR m.created_at >= $2)
      AND (NOT EXISTS (SELECT 1 FROM mark)
           OR m.created_at > (SELECT created_at FROM mark)
           OR (m.created_at = (SELECT created_at FROM mark)
               AND m.id > (SELECT id FROM mark)))
    ORDER BY m.created_at ASC, m.id ASC
    LIMIT $4";
