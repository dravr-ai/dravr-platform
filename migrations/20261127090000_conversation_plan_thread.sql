-- ABOUTME: Marks the one conversation that is an athlete's own plan thread in a tenant, their Plan tab's Dravr tab (carnet#1841)
-- ABOUTME: Additive only: a flag plus a partial unique index; no row is marked until the athlete opens their plan thread

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A self-coached athlete talks about their own plan with their agent in a
-- thread of its own, beside Home's personal thread, so Home stays free of
-- plan traffic. The flag names that thread; the server opens it the first
-- time the athlete asks for it, so nothing is backfilled here. A plan thread
-- is the owner's alone: in no group, about no other athlete, never the
-- personal thread. SQLite cannot add that CHECK to an existing table without
-- rebuilding it; the server's designation enforces it, as the PostgreSQL
-- CHECK does there.
ALTER TABLE chat_conversations ADD COLUMN plan_thread INTEGER NOT NULL DEFAULT 0; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- One plan thread per athlete per tenant: a second designation racing the
-- first fails here, and the loser reads the winner.
CREATE UNIQUE INDEX IF NOT EXISTS idx_chat_conversations_plan_thread
    ON chat_conversations (tenant_id, user_id) WHERE plan_thread = 1;
