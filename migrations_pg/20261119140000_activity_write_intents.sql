-- ABOUTME: activity_write_intents — a walk step's write to the activity store, recorded before it lands (PostgreSQL)
-- ABOUTME: Cleared once the step's check after the write was read; a row left behind is settled by the resume sweep (registre#1555)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A walk step checks its ledger row and the purge attestations after it
-- writes its page; when that check stays unreadable, the step cannot tell
-- whether a purge of its key landed in between, and with its row gone
-- nothing retries it. The step writes this row before its write, in the
-- same database as the ledger, and deletes it once its check was read. A
-- row still here is settled by the resume sweep once the attestations read
-- again: a purge of the key that deleted activities since `purges_since`,
-- beyond the ones attested before the step's first check, takes the page
-- back, only the rows still carrying the step's write; none drops the row.
--   id               the intent's id
--   tenant_id        the step's tenant
--   user_id          the athlete
--   provider         the key the step wrote under
--   kind             the step's job kind (history_walk, detail_walk)
--   page_ids         JSON array: every activity id the step wrote
--   added_ids        JSON array: those the store neither served nor recorded
--                    before the write, what a retention expiry takes back
--   written_at_us    the write's sync stamp (unix microseconds): a copy
--                    stamped later is another writer's and stays
--   purges_since_ms  the attestations are read from here (unix ms)
--   attested_before  JSON array: the purge reasons read before the step's
--                    first check, which are not the step's to answer
--   created_at_ms    when the step recorded it
--   due_at_ms        when the sweep may settle it: a grace past the step's
--                    own check, then pushed back after each failed attempt
--   attempts         failed attempts so far
-- Additive; no backfill.
CREATE TABLE IF NOT EXISTS activity_write_intents (
    id TEXT PRIMARY KEY,
    tenant_id UUID NOT NULL,
    user_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    kind TEXT NOT NULL,
    page_ids TEXT NOT NULL,
    added_ids TEXT NOT NULL,
    written_at_us BIGINT NOT NULL,
    purges_since_ms BIGINT NOT NULL,
    attested_before TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    due_at_ms BIGINT NOT NULL,
    attempts BIGINT NOT NULL DEFAULT 0
);

-- The sweep: intents due, oldest first.
CREATE INDEX IF NOT EXISTS idx_activity_write_intents_due
    ON activity_write_intents (due_at_ms);
