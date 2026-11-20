-- ABOUTME: When a history walk's first wellness fill completed, on its job row (Postgres)
-- ABOUTME: Additive only: one nullable column on activity_backfill_jobs; nothing is renamed or dropped (carnet#1661)
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the matching SQLite migration. When the walk's first wellness fill
-- completed (unix ms); NULL while it has not.
ALTER TABLE activity_backfill_jobs ADD COLUMN IF NOT EXISTS wellness_filled_at_ms BIGINT;
