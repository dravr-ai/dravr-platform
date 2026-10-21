-- ABOUTME: A claimed job or queued message carries the token of the claim that holds it
-- ABOUTME: Renewing, finishing or settling it requires that token, so a worker whose lease lapsed changes nothing

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A lease said until when a row was held, never by whom. A worker whose lease
-- lapsed while another re-claimed the row still renewed it, and its finish
-- deleted or overwrote the new holder's work. Each claim now writes a fresh
-- token; every later write by that worker names it.

ALTER TABLE memory_extraction_jobs ADD COLUMN IF NOT EXISTS lease_token TEXT;
ALTER TABLE activity_backfill_jobs ADD COLUMN IF NOT EXISTS lease_token TEXT;
ALTER TABLE worker_runs ADD COLUMN IF NOT EXISTS lease_token TEXT;
ALTER TABLE platform_post_claims ADD COLUMN IF NOT EXISTS lease_token TEXT;

-- The outbound retry queue had no claim at all: every instance read the same
-- due rows and could send an athlete the same message twice.
ALTER TABLE messaging_outbound_queue ADD COLUMN IF NOT EXISTS lease_token TEXT;
ALTER TABLE messaging_outbound_queue ADD COLUMN IF NOT EXISTS leased_until_ms BIGINT NOT NULL DEFAULT 0;
