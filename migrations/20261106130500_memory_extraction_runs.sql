-- ABOUTME: One row per finished post-turn memory extraction run: how it ended and how many facts it wrote
-- ABOUTME: Measures the worker directly; user_facts has other writers and an empty run leaves no trace there
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The admin memory worker screen inferred the extractor's health from
-- user_facts (carnet#1433): an empty run writes nothing there, an athlete
-- approving a draft bumps updated_at, and a job row is deleted on success, so
-- no store could say "it ran and found nothing" or "it failed". This ledger
-- costs one insert per run and carries no user id, so nothing in it is about
-- an athlete. The resume sweep prunes it past its retention.
CREATE TABLE IF NOT EXISTS memory_extraction_runs (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('persisted', 'empty', 'failed')),
    facts_persisted INTEGER NOT NULL DEFAULT 0,
    finished_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_memory_extraction_runs_tenant_finished
    ON memory_extraction_runs(tenant_id, finished_at_ms);

CREATE INDEX IF NOT EXISTS idx_memory_extraction_runs_finished
    ON memory_extraction_runs(finished_at_ms);
