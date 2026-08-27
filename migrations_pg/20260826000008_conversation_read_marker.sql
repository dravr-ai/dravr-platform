-- ABOUTME: PostgreSQL per-participant read marker — the source of every unread count
-- ABOUTME: Mirrors migrations/20260826000008_conversation_read_marker.sql with PG-native types
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- last_read_at is the created_at of the newest row the participant has seen,
-- advanced monotonically by the read routes and by the pipeline after it
-- persists the participant's own turn. It lives on the membership row rather
-- than the conversation because "unread" is a question about one participant:
-- two members of the same thread each keep their own marker. Cleared to NULL
-- by "mark unread".
ALTER TABLE conversation_participants ADD COLUMN IF NOT EXISTS last_read_at TIMESTAMPTZ;

-- The list page asks three questions per conversation — how many turns, how
-- many after the marker, and which row is newest — each scoped to one
-- conversation and ordered by created_at. SQLite has carried this composite
-- index since 20250120000017; the PostgreSQL schema only indexed the
-- conversation, so every one of those subqueries sorted the thread on read.
CREATE INDEX IF NOT EXISTS idx_chat_messages_conversation_created
    ON chat_messages(conversation_id, created_at);
