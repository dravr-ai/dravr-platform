-- ABOUTME: feedback_messages — what an athlete wrote to the Dravr team in the in-app feedback thread (PostgreSQL)
-- ABOUTME: Mirrors migrations/20261028170000_feedback_messages.sql with native uuid ids (carnet#1235)
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- One row per message, oldest first per athlete and tenant. The thread is
-- the athlete's own: every read is keyed by tenant_id and user_id, and the
-- account delete takes the rows along through the user_id foreign key.
--   screen                 the web route the thread was opened from
--   app_version, locale    the client's build and language when it sent
--   about_conversation_id  the conversation of the agent reply the message
--   about_message_id       is about, when opened from a thumbs-down
--   created_at_ms          epoch milliseconds, identical on both engines
CREATE TABLE IF NOT EXISTS feedback_messages (
    id TEXT PRIMARY KEY,
    tenant_id UUID NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    body TEXT NOT NULL,
    screen TEXT,
    app_version TEXT,
    locale TEXT,
    about_conversation_id TEXT,
    about_message_id TEXT,
    created_at_ms BIGINT NOT NULL
);

-- The thread read: one athlete's messages in a tenant, newest window first.
CREATE INDEX IF NOT EXISTS idx_feedback_messages_thread
    ON feedback_messages (tenant_id, user_id, created_at_ms);
