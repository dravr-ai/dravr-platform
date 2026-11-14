-- ABOUTME: The sport a detail walk reads, beside the window and progress columns it reuses from the history walk (SQLite)
-- ABOUTME: Additive only: one nullable column on activity_backfill_jobs; nothing is renamed or dropped (carnet#1483)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A detail walk ('detail_walk' in `kind`) reads, newest first, the stored
-- activities of a window whose segment efforts were never read, and fills
-- each through a backend that reads them. It reuses the columns the history
-- walk added in 20261112130000:
--   after_ts / before_ts       the window floor, and the walk's cursor, which
--                              starts at the window's top and moves down
--   months_total / months_done the activities to read, and those read so far
--                              (the columns keep their first names; the unit
--                              is the job kind's)
--   rows_landed, secs_per_unit efforts stored so far, and the measured pace
-- The one thing it needs that no column holds is the sport it was asked for:
--   walk_sport  the sport type the walk reads, NULL for every sport
ALTER TABLE activity_backfill_jobs ADD COLUMN walk_sport TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
