-- ABOUTME: garmin_push_inbox — every batch Garmin pushes, written before the 200 and worked through under a lease
-- ABOUTME: Garmin does not resend a batch it saw answered, so a push survives the instance that received it only here
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The Garmin Connect Developer Program pushes each summary type to its own
-- URL and wants a 200 within thirty seconds, before any processing. Once it
-- has its 200 it never sends the batch again, so a batch held only in memory
-- is lost with the instance (a deploy, an idle scaledown). The raw body is
-- written here first; a worker then claims each row under a lease, maps its
-- records into Dravr's stores, and retries with backoff when that fails.
--
-- A batch names several Garmin users, so the row carries no user or tenant:
-- each record's owner is resolved when the row is processed. A processed row
-- keeps its bookkeeping, its payload blanked, until the retention sweep
-- deletes it.
CREATE TABLE IF NOT EXISTS garmin_push_inbox (
    id TEXT PRIMARY KEY,
    summary_type TEXT NOT NULL,
    received_at_ms INTEGER NOT NULL,
    payload TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'done', 'failed')),
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    processed_at_ms INTEGER,
    leased_until_ms INTEGER NOT NULL DEFAULT 0,
    lease_token TEXT
);

CREATE INDEX IF NOT EXISTS idx_garmin_push_inbox_due
    ON garmin_push_inbox(status, leased_until_ms, received_at_ms);
CREATE INDEX IF NOT EXISTS idx_garmin_push_inbox_processed
    ON garmin_push_inbox(processed_at_ms);
