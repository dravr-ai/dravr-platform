-- ABOUTME: One row per post-turn memory extraction still owed, recorded before the extraction is spawned
-- ABOUTME: Lets an extraction an instance died inside be re-run by the next instance instead of silently lost
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The extraction runs after the turn's request has answered, queued behind
-- a semaphore and the LLM transport, so a deploy landing inside it lost the
-- athlete's facts for that turn with only a log line (carnet#461). The row
-- is written first and deleted on success; the resume sweep re-runs any row
-- whose lease has lapsed. payload is the request as JSON; tenant_id is a
-- column because TenantId is deliberately not deserialisable.
CREATE TABLE IF NOT EXISTS memory_extraction_jobs (
    id TEXT PRIMARY KEY,
    tenant_id UUID NOT NULL,
    payload TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    leased_until_ms BIGINT NOT NULL DEFAULT 0,
    attempts BIGINT NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_memory_extraction_jobs_due
    ON memory_extraction_jobs(leased_until_ms, created_at_ms);
