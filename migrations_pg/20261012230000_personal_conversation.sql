-- ABOUTME: Marks the one conversation that is an athlete's personal thread in a tenant (carnet#840, Personal Home step 2)
-- ABOUTME: Additive only: a flag plus a partial unique index; rows written before it are not personal until the server adopts one

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Home, the coach's own posts and "+" all land in one continuous thread per
-- athlete. Until now each picked its own: Home took the newest 1:1 row the
-- client had loaded, a platform post took the newest one on the server, and
-- "+" or /reset forged another. The flag names the thread once; the server
-- adopts the athlete's newest in-app 1:1 thread the first time it is asked,
-- so nothing is backfilled here.
ALTER TABLE chat_conversations ADD COLUMN IF NOT EXISTS personal BOOLEAN NOT NULL DEFAULT FALSE;

-- One personal thread per athlete per tenant: a second designation racing the
-- first fails here, and the loser reads the winner.
CREATE UNIQUE INDEX IF NOT EXISTS idx_chat_conversations_personal
    ON chat_conversations (tenant_id, user_id) WHERE personal;
