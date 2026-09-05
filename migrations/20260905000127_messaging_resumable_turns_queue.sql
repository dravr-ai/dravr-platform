-- ABOUTME: Every messaging turn becomes a resumable row at ingress, started through a runner that may hand it to Cloud Tasks
-- ABOUTME: Adds the per-enqueue sequence a Cloud Tasks task name carries, and the conversation index the ordered claim reads
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A Cloud Tasks task name stays unusable for up to 24 hours after the task
-- executed or was deleted, so a turn re-enqueued by the sweep needs a name the
-- queue has never seen: the row counts its enqueues and the name carries the
-- count (registre#126).
ALTER TABLE messaging_resumable_turns ADD COLUMN enqueue_seq INTEGER NOT NULL DEFAULT 0; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- The claim refuses a turn while an older turn of the same conversation is
-- still on file, which keeps replies in order across instances; this is the
-- index that lookup walks.
CREATE INDEX IF NOT EXISTS idx_messaging_resumable_turns_conversation
    ON messaging_resumable_turns(tenant_id, conversation, created_at_ms);
