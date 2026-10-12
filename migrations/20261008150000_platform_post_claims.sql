-- ABOUTME: One row per post the coach made to an athlete on its own initiative, keyed by what the post was for
-- ABOUTME: The durable once-claim that keeps a server-initiated coach post to one per athlete and purpose
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The coach speaks first once an athlete's data lands (carnet#831), and that
-- post is owed once: a second provider connected, a reconnect, the same
-- connect's backfill finishing on two replicas, each fires the same trigger.
-- The primary key is what makes the post happen once: the first trigger to
-- insert the row posts, every later one finds it taken. A trigger that ended
-- up posting nothing deletes its row, so the next landing tries again.
--
-- purpose names what the post was for: 'first_sync', owed once ever, or
-- 'check_in', owed once per day the athlete's cadence posts on — its row is
-- retaken once claimed_at_ms predates the athlete's local midnight, so
-- replicas still post once and the athlete keeps a single row. claimed_at_ms
-- is epoch milliseconds, identical on both engines. The row goes with the
-- athlete.
CREATE TABLE IF NOT EXISTS platform_post_claims (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    purpose TEXT NOT NULL,
    claimed_at_ms INTEGER NOT NULL,
    PRIMARY KEY (tenant_id, user_id, purpose)
);
