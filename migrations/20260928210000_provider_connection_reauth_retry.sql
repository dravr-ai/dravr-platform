-- ABOUTME: Stamp each retry of a scrape session flagged needs_reauth, so a flag one failed read set is revisited on a throttle
-- ABOUTME: Read and claimed by the capture sweep and Athlete Home; OAuth connections never carry a value here
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- When the platform last re-tried a flagged scrape session's stored cookies.
-- NULL until the first retry. A retry is due once both this and
-- status_changed_at (the flag itself, which was an attempt too) are older
-- than the retry interval, so a flag from a new episode is never retried
-- early by a stamp left from an old one.
ALTER TABLE provider_connections ADD COLUMN reauth_retry_at TEXT;  -- idempotency-ok: SQLite lacks ADD COLUMN IF NOT EXISTS; column is new here
