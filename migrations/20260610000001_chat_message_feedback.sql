-- Per-message thumbs up/down feedback on assistant chat replies.
-- One row per (message, user); the rating toggles up/down and an optional
-- free-text comment captures "what went wrong?" on a thumbs-down. conversation_id
-- is denormalized so the messages-list load and future analytics can scope by
-- conversation/coach without re-joining chat_messages.
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

CREATE TABLE IF NOT EXISTS chat_message_feedback (
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES chat_messages(id) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    rating TEXT NOT NULL CHECK (rating IN ('up', 'down')),
    comment TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(message_id, user_id)
);
CREATE INDEX IF NOT EXISTS idx_chat_message_feedback_conversation ON chat_message_feedback(conversation_id, user_id);
CREATE INDEX IF NOT EXISTS idx_chat_message_feedback_rating ON chat_message_feedback(rating, created_at);
