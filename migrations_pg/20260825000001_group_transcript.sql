-- ABOUTME: Surface-neutral group room transcript — one row per member or coach utterance
-- ABOUTME: Fanned out at turn persistence (any surface) plus messaging ambient capture
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- group_transcript_entries: the shared room view of a coaching group.
-- Turn rows (speaker 'member' + the 'coach' reply) are appended by
-- chat-pipeline persistence whenever the conversation is group-bound,
-- whatever surface the turn arrived on; 'member' rows with no
-- source_conversation_id are ambient room chatter captured by the messaging
-- ingress. tenant_id records the tenant the writing conversation/session
-- lives under; reads are keyed on group_id because group membership is
-- cross-tenant, same as coaching_group_members.
CREATE TABLE IF NOT EXISTS group_transcript_entries (
    id UUID PRIMARY KEY,
    group_id UUID NOT NULL REFERENCES coaching_groups(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    author_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    speaker TEXT NOT NULL CHECK (speaker IN ('member', 'coach')),
    content TEXT NOT NULL,
    source_conversation_id VARCHAR(255),
    source_message_id VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_group_transcript_group_created
    ON group_transcript_entries(group_id, created_at);
