-- ABOUTME: When a history walk's first wellness fill completed, on its job row (SQLite)
-- ABOUTME: Additive only: one nullable column on activity_backfill_jobs; nothing is renamed or dropped (carnet#1661)
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A Garmin or COROS athlete's first wellness copy (the year before their push
-- grant) is counted in a day allowance of its own,
-- sciotte_reads.wellness_first_fill_daily. Once that first fill completes,
-- a later copy of the walk counts in the ordinary background caps. This keeps
-- when it completed (unix ms); NULL while it has not. A new ask of the row
-- keeps it.
ALTER TABLE activity_backfill_jobs ADD COLUMN wellness_filled_at_ms INTEGER; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
