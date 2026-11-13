-- ABOUTME: One backfill job row per (user, provider, kind): a history walk, a live read and a backfill no longer share one slot
-- ABOUTME: Rebuilds activity_backfill_jobs (SQLite) with kind NOT NULL in the unique key, plus the walk's progress and the asking turn

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Until now a job row was keyed on (user_id, provider), and `kind` was NULL
-- for an ordinary backfill or 'history_import' for the retired one-time
-- session import. 20261112120000 deleted every row the import wrote (a kind
-- or an outcome set), so each row left here is an ordinary backfill. The job
-- model now has three kinds, each its own row beside the others:
--
--   backfill      a chat's or the capture rail's deep window (was NULL)
--   history_walk  the whole-history walk, back to the account's floor
--   live_read     a chat read that outran its turn budget; a newer ask while
--                 it is owed rebinds the turn it answers
--
-- `kind` is NOT NULL so it can sit in the unique key: NULLs never collide in
-- a UNIQUE. The vocabulary is enforced where the row is written and read, so
-- a new kind needs no table rebuild.
--
-- New columns:
--   months_total      how many months the walk has to cover, NULL when its
--                     floor is not known yet
--   months_done       months walked so far
--   rows_landed       activities written so far
--   secs_per_unit     measured seconds per walked month, NULL until measured
--   turn_id           the conversation turn a live read answers
--   asked_message_id  the user message that asked it
--   walk_backend      the backend a history walk's current phase reads: NULL
--                     reads `provider`, the walk's own source; the API grant
--                     once the session's phase handed off below the oldest
--                     row it reached
--
-- SQLite cannot change an inline UNIQUE, so the table is rebuilt with every
-- column, default and index it carries (20260918000003 plus the lease-token
-- and outcome migrations), and every row is copied as a 'backfill'. No
-- foreign key points at activity_backfill_jobs, and none leaves it.
CREATE TABLE activity_backfill_jobs_new ( -- idempotency-ok: table rebuild — SQLite cannot change an inline UNIQUE; sqlx applies each migration exactly once
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'backfill',
    after_ts INTEGER,
    before_ts INTEGER,
    fetch_limit INTEGER,
    conversation_id TEXT,
    created_at_ms INTEGER NOT NULL,
    leased_until_ms INTEGER NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0,
    lease_token TEXT,
    outcome TEXT,
    months_total INTEGER,
    months_done INTEGER NOT NULL DEFAULT 0,
    rows_landed INTEGER NOT NULL DEFAULT 0,
    secs_per_unit REAL,
    turn_id TEXT,
    asked_message_id TEXT,
    walk_backend TEXT,
    UNIQUE (user_id, provider, kind)
);
INSERT INTO activity_backfill_jobs_new
    (id, tenant_id, user_id, provider, kind, after_ts, before_ts, fetch_limit,
     conversation_id, created_at_ms, leased_until_ms, attempts, lease_token, outcome)
SELECT id, tenant_id, user_id, provider, 'backfill',
       after_ts, before_ts, fetch_limit, conversation_id, created_at_ms,
       leased_until_ms, attempts, lease_token, outcome
FROM activity_backfill_jobs;
DROP TABLE activity_backfill_jobs; -- idempotency-ok: table rebuild — the rebuilt table is renamed over it on the next line
ALTER TABLE activity_backfill_jobs_new RENAME TO activity_backfill_jobs;

CREATE INDEX IF NOT EXISTS idx_activity_backfill_jobs_due
    ON activity_backfill_jobs(leased_until_ms, created_at_ms);
