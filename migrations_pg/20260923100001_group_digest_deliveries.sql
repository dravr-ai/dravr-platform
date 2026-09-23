-- ABOUTME: One row per coaching group per local week its weekly digest was sent for, and who holds that send
-- ABOUTME: Keeps the digest to once per group per week across ticks, restarts and instances
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The group weekly digest used to go out whenever its 168-hour worker ticked,
-- so its hour was wherever the first tick after a deploy happened to land
-- (01:39 local, carnet#519). It now ticks every fifteen minutes and sends at a
-- fixed morning slot in the group's own zone, so "already sent this week" has
-- to be a fact about the group rather than about the worker.
--
-- week_key is the ISO week in the group's calendar ('2026-W39'). A tick claims
-- the week by inserting the row with leased_until_ms set; finished_at_ms is
-- stamped once the attempt is over, sent or not, and no later claim for that
-- week succeeds. Only an instance that died mid-send leaves the lease to run
-- out, and the next tick in the slot retries. Both are epoch milliseconds,
-- identical on both engines.
CREATE TABLE IF NOT EXISTS group_digest_deliveries (
    tenant_id TEXT NOT NULL,
    group_id UUID NOT NULL REFERENCES coaching_groups(id) ON DELETE CASCADE,
    week_key TEXT NOT NULL,
    leased_until_ms BIGINT NOT NULL DEFAULT 0,
    finished_at_ms BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (tenant_id, group_id, week_key)
);
