-- ABOUTME: activity_conversations — the thread an activity's view opened, one per (tenant, user, provider, activity)
-- ABOUTME: Reopening the activity on any device resumes that thread; the row goes with the conversation it names

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- An activity's view carries a chat about the activity. Its first question
-- opens a conversation, and every later question, on this visit or a return
-- to the same activity from any device, belongs in that one thread rather
-- than a second conversation about the same workout. The link is kept here,
-- keyed like the activity's other per-activity rows (activity_route_tracks):
-- by the provider key the cached activity is stored under, so the
-- provider-disconnect purge reaches it with the same statement.
--
-- A conversation deleted takes its link with it (ON DELETE CASCADE), and a
-- read joins the conversation back to the same user and tenant, so a link
-- never answers with a thread that is gone or someone else's. Ids are TEXT,
-- like cached_activities.

CREATE TABLE IF NOT EXISTS activity_conversations (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    activity_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY (tenant_id, user_id, provider, activity_id)
);

CREATE INDEX IF NOT EXISTS idx_activity_conversations_user_tenant
    ON activity_conversations(user_id, tenant_id);

CREATE INDEX IF NOT EXISTS idx_activity_conversations_conversation
    ON activity_conversations(conversation_id);
