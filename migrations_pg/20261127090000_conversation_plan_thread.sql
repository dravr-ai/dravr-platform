-- ABOUTME: Marks the one conversation that is an athlete's own plan thread in a tenant, their Plan tab's Dravr tab (carnet#1841)
-- ABOUTME: Additive only: a flag, a partial unique index and a CHECK; no row is marked until the athlete opens their plan thread

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A self-coached athlete talks about their own plan with their agent in a
-- thread of its own, beside Home's personal thread, so Home stays free of
-- plan traffic. The flag names that thread; the server opens it the first
-- time the athlete asks for it, so nothing is backfilled here.
ALTER TABLE chat_conversations ADD COLUMN IF NOT EXISTS plan_thread BOOLEAN NOT NULL DEFAULT FALSE;

-- One plan thread per athlete per tenant: a second designation racing the
-- first fails here, and the loser reads the winner.
CREATE UNIQUE INDEX IF NOT EXISTS idx_chat_conversations_plan_thread
    ON chat_conversations (tenant_id, user_id) WHERE plan_thread;

-- A plan thread is the owner's alone: in no group, about no other athlete,
-- and never their personal thread. Every existing row has plan_thread FALSE,
-- so the constraint validates without a rewrite of what it reads.
ALTER TABLE chat_conversations DROP CONSTRAINT IF EXISTS chat_conversations_plan_thread_check;
ALTER TABLE chat_conversations
    ADD CONSTRAINT chat_conversations_plan_thread_check
    CHECK (NOT plan_thread OR (group_id IS NULL AND subject_user_id IS NULL AND NOT personal));
