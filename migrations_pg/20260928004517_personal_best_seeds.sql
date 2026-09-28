-- ABOUTME: How far the one-time walk of an athlete's provider history has measured their past runs for best efforts
-- ABOUTME: A personal record is announced only once that walk has reached their first activity

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- One row per (athlete, tenant, provider) whose history the seed has started
-- walking, newest activity first. cursor_before is the exclusive upper bound
-- (unix seconds) of the next page it lists: every activity that started at or
-- after it was measured or skipped, so a restart lists from there. completed_at
-- is set when a listing past the oldest activity came back empty; until then
-- the stored bests cover part of the history and no record is announced.
-- provider names the history walked, so a provider's disconnect purge resets
-- the walk along with the bests it produced.
CREATE TABLE IF NOT EXISTS personal_best_seeds (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id UUID NOT NULL,
    provider TEXT NOT NULL,
    cursor_before BIGINT,
    completed_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (user_id, tenant_id, provider)
);
