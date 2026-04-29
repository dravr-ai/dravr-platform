-- ABOUTME: Phase 5B — billing_events idempotency log for the webhook handler (PostgreSQL)
-- ABOUTME: One row per (provider, event_id); webhook handler skips replay when the row exists
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

CREATE TABLE IF NOT EXISTS billing_events (
    provider TEXT NOT NULL,
    event_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (provider, event_id)
);

CREATE INDEX IF NOT EXISTS idx_billing_events_type ON billing_events(event_type);
