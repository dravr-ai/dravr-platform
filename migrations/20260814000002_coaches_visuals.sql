-- ABOUTME: Adds visuals to coaches — which inline visual kinds a coach may embed.
-- ABOUTME: Comma-separated wire names (chart,table); NULL/empty means none.
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- ============================================================================
-- Add visuals grant for inline chart/table blocks
-- ============================================================================

-- Which inline visuals this coach is permitted to embed in a reply, as a
-- comma-separated list of wire names ("chart", "table"). NULL or empty means
-- the coach is never shown the visual contract, so it never emits a block.
--
-- Distinct from output_schema: that declares "my whole reply is this object",
-- while this grants embedding several visuals INSIDE ordinary prose. A coach
-- can legitimately have both, neither, or one.
--
-- Stored as text rather than a join table because it is a closed two-value set
-- read on every turn; a table would add a query to the hot path to model a flag.
ALTER TABLE coaches ADD COLUMN visuals TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
