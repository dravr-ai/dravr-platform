-- ABOUTME: PostgreSQL per-message thumbs up/down feedback on assistant chat replies
-- ABOUTME: Mirrors migrations/20260610000001_chat_message_feedback.sql with PG-native types
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

CREATE TABLE IF NOT EXISTS chat_message_feedback (
    id TEXT PRIMARY KEY,
    message_id VARCHAR(255) NOT NULL REFERENCES chat_messages(id) ON DELETE CASCADE,
    conversation_id VARCHAR(255) NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
    user_id UUID NOT NULL,
    tenant_id TEXT NOT NULL,
    rating TEXT NOT NULL CHECK (rating IN ('up', 'down')),
    comment TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE(message_id, user_id)
);
CREATE INDEX IF NOT EXISTS idx_chat_message_feedback_conversation ON chat_message_feedback(conversation_id, user_id);
CREATE INDEX IF NOT EXISTS idx_chat_message_feedback_rating ON chat_message_feedback(rating, created_at);
