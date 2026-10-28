-- ABOUTME: cached_write_1h_tokens column on llm_usage: the 1-hour share of a call's cache write
-- ABOUTME: Anthropic bills a 1-hour write at 2x input, a 5-minute one at 1.25x; one count cannot price both
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- embacle 0.42.0 marks the tools and system cache breakpoints for one hour
-- (ANTHROPIC_PREFIX_CACHE_TTL=1h) and reports usage.cache_creation's 1-hour
-- part apart. cached_write_tokens keeps counting every write; this column
-- holds the part of it written to a 1-hour entry, so cost_usd recomputed
-- from a row bills that part at 2x input instead of 1.25x.
--
-- Additive, defaulting to 0: a row written before this migration prices
-- every write at the 5-minute rate, which is what it was billed at.
ALTER TABLE llm_usage ADD COLUMN cached_write_1h_tokens INTEGER NOT NULL DEFAULT 0; -- idempotency-ok: SQLite has no ADD COLUMN IF NOT EXISTS; brand-new column, PG mirror uses IF NOT EXISTS
