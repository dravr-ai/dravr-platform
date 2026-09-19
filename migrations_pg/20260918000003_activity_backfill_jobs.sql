-- ABOUTME: One row per historical activity backfill still owed, recorded before the scrape is spawned
-- ABOUTME: Lets a backfill an instance died inside be re-run by the next instance so the promised follow-up arrives
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A backfill scrapes for minutes on an instance Cloud Run reads as idle the
-- moment the chat turn's request ended, and the athlete is told the finished
-- window will come back on its own and not to ask again. A deploy or an idle
-- scaledown mid-scrape used to lose the whole job with no trace (carnet#460).
-- The row is written first and deleted when the run completes or needs the
-- athlete to reconnect; the resume sweep re-runs any row whose lease lapsed.
-- One row per (user, provider): a second ask while one is owed is the same job.
CREATE TABLE IF NOT EXISTS activity_backfill_jobs (
    id TEXT PRIMARY KEY,
    tenant_id UUID NOT NULL,
    user_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    after_ts BIGINT,
    before_ts BIGINT,
    fetch_limit BIGINT,
    conversation_id TEXT,
    created_at_ms BIGINT NOT NULL,
    leased_until_ms BIGINT NOT NULL DEFAULT 0,
    attempts BIGINT NOT NULL DEFAULT 0,
    UNIQUE (user_id, provider)
);

CREATE INDEX IF NOT EXISTS idx_activity_backfill_jobs_due
    ON activity_backfill_jobs(leased_until_ms, created_at_ms);
