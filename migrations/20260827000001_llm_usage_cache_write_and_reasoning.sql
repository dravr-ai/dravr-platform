-- ABOUTME: cached_write_tokens + reasoning_tokens columns on llm_usage
-- ABOUTME: Both counts are on the provider wire and were discarded; both understate cost when dropped
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- embacle 0.22.0 keeps three optional counts its parsers used to step past.
-- Two of them change what a call costs:
--
-- cached_write_tokens: prompt tokens written INTO the provider's context
--     cache. Anthropic bills these at 1.25x the input rate -- a premium, not
--     a discount. Folded into the fresh-prompt count they billed at 1.0x, so
--     every cold prefix was understated. A live Copilot ACP turn
--     (claude-opus-4.8, 2026-08-27) reported 12,540 of them on one call.
-- reasoning_tokens: "thought" tokens billed at the output rate and excluded
--     from completion_tokens by every provider that reports them apart, so
--     dropping them charged nothing at all for that output.
--
-- Both default to 0, which prices identically to the old behaviour for rows
-- written before this migration -- no backfill can recover counts that were
-- never persisted, and a 0 here reads as "not reported", same as cached_tokens.
ALTER TABLE llm_usage ADD COLUMN cached_write_tokens INTEGER NOT NULL DEFAULT 0; -- idempotency-ok: SQLite has no ADD COLUMN IF NOT EXISTS; brand-new column, PG mirror uses IF NOT EXISTS
ALTER TABLE llm_usage ADD COLUMN reasoning_tokens INTEGER NOT NULL DEFAULT 0; -- idempotency-ok: SQLite has no ADD COLUMN IF NOT EXISTS; brand-new column, PG mirror uses IF NOT EXISTS
