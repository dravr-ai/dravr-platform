-- ABOUTME: One backfill job row per (user, provider, kind): a history walk, a live read and a backfill no longer share one slot
-- ABOUTME: Backfills kind, makes it NOT NULL, swaps UNIQUE (user_id, provider) for one with kind, adds the walk's progress columns

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the matching SQLite migration for the full rationale. 20261112120000
-- deleted every row the retired history import wrote, so each row left has a
-- NULL kind, an ordinary backfill: it becomes 'backfill', and `kind` is then
-- NOT NULL so it can sit in the unique key.
-- The progress column is `rows_landed`, never `rows`, a keyword in
-- PostgreSQL.
--
-- The old key was declared inline in 20260918000003 as
-- UNIQUE (user_id, provider), which PostgreSQL names
-- activity_backfill_jobs_user_id_provider_key; the migration test asserts
-- that name before this runs and that only the new key stands after it.
-- Every statement is guarded, so a second pass is a no-op.
UPDATE activity_backfill_jobs SET kind = 'backfill' WHERE kind IS NULL;
ALTER TABLE activity_backfill_jobs ALTER COLUMN kind SET DEFAULT 'backfill';
ALTER TABLE activity_backfill_jobs ALTER COLUMN kind SET NOT NULL;

ALTER TABLE activity_backfill_jobs ADD COLUMN IF NOT EXISTS months_total BIGINT;
ALTER TABLE activity_backfill_jobs ADD COLUMN IF NOT EXISTS months_done BIGINT NOT NULL DEFAULT 0;
ALTER TABLE activity_backfill_jobs ADD COLUMN IF NOT EXISTS rows_landed BIGINT NOT NULL DEFAULT 0;
ALTER TABLE activity_backfill_jobs ADD COLUMN IF NOT EXISTS secs_per_unit DOUBLE PRECISION;
ALTER TABLE activity_backfill_jobs ADD COLUMN IF NOT EXISTS turn_id TEXT;
ALTER TABLE activity_backfill_jobs ADD COLUMN IF NOT EXISTS asked_message_id TEXT;
ALTER TABLE activity_backfill_jobs ADD COLUMN IF NOT EXISTS walk_backend TEXT;

ALTER TABLE activity_backfill_jobs DROP CONSTRAINT IF EXISTS activity_backfill_jobs_user_id_provider_key;
ALTER TABLE activity_backfill_jobs DROP CONSTRAINT IF EXISTS activity_backfill_jobs_user_kind_key;
ALTER TABLE activity_backfill_jobs
    ADD CONSTRAINT activity_backfill_jobs_user_kind_key UNIQUE (user_id, provider, kind);
