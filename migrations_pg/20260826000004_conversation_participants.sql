-- ABOUTME: PostgreSQL conversation participants — every user who can read and post in a chat conversation
-- ABOUTME: Mirrors migrations/20260826000004_conversation_participants.sql with PG-native types
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- conversation_participants: who is in a conversation. Every read and write
-- path on chat_conversations / chat_messages that used to filter on the
-- owning user_id now asks this table instead, so a non-owner participant
-- reads and posts in the thread exactly like the owner does.
--
-- tenant_id always equals the conversation's tenant: a participant is a
-- member of that tenant, and the add route refuses anyone who is not. The
-- owner keeps two privileges the role column encodes — deleting the
-- conversation and never being removed from it.
CREATE TABLE IF NOT EXISTS conversation_participants (
    conversation_id VARCHAR(255) NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id VARCHAR(255) NOT NULL,
    role VARCHAR(50) NOT NULL CHECK (role IN ('owner', 'member')),
    added_by UUID NOT NULL,
    added_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (conversation_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_conversation_participants_user_tenant
    ON conversation_participants(user_id, tenant_id);

-- Backfill: every pre-existing conversation gets its owner as a participant,
-- so the membership predicate never locks an athlete out of their own thread.
INSERT INTO conversation_participants
    (conversation_id, user_id, tenant_id, role, added_by, added_at)
SELECT id, user_id, tenant_id, 'owner', user_id, created_at
FROM chat_conversations
ON CONFLICT (conversation_id, user_id) DO NOTHING;
