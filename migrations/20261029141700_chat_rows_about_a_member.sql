-- ABOUTME: Names the group member a chat row or compaction summary is about, so withdrawal and deletion reach it (carnet#1174)
-- ABOUTME: Additive only: one nullable column on each table plus partial indexes; rows written before it stay NULL

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A human coach's group chat holds what the agent read about the members the
-- coach asked about: tool results and the replies drawn from them. The rows
-- are keyed to the coach's conversation, so nothing told one athlete's rows
-- apart. Each such row now names the member it read: a member who stops
-- sharing or leaves is no longer replayed into a prompt, and the rows go with
-- the member's account, which the foreign key holds and the user delete
-- states (SQLITE_USER_PURGE).
--
-- NULL is a row about nobody but the conversation's own participants.
ALTER TABLE chat_messages ADD COLUMN about_user_id TEXT REFERENCES users(id) ON DELETE CASCADE; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- A summary of such rows names the one member they read.
ALTER TABLE compaction_blocks ADD COLUMN about_user_id TEXT REFERENCES users(id) ON DELETE CASCADE; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- A turn lists the members its conversation's rows are about before it loads
-- the history; only stamped rows are indexed, so the read skips the rest.
CREATE INDEX IF NOT EXISTS idx_chat_messages_conversation_about
    ON chat_messages(conversation_id, about_user_id) WHERE about_user_id IS NOT NULL;

-- The account delete finds a member's rows by the member.
CREATE INDEX IF NOT EXISTS idx_chat_messages_about_user
    ON chat_messages(about_user_id) WHERE about_user_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_compaction_blocks_about_user
    ON compaction_blocks(about_user_id) WHERE about_user_id IS NOT NULL;
