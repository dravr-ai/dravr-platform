-- ABOUTME: The sport a detail walk reads, its window's top and when it ended, beside the columns it reuses (SQLite)
-- ABOUTME: Additive only: three nullable columns on activity_backfill_jobs; nothing is renamed or dropped (carnet#1483)

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
-- What it needs that no column holds:
--   walk_sport     the sport type the walk reads, NULL for every sport
--   window_top_ts  the window's top as the ask that recorded the row named it
--                  (unix seconds): `before_ts` moves down as the cursor, and an
--                  ended walk's sits at its floor, so the top is kept here
--   ended_at_ms    when a detail walk ended walked (unix ms), NULL while owed
--                  or after any other end: an ask within a week over a window
--                  a walk ended on records no fresh walk, which would only
--                  re-read the activities the backend could not read
ALTER TABLE activity_backfill_jobs ADD COLUMN walk_sport TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE activity_backfill_jobs ADD COLUMN window_top_ts INTEGER; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE activity_backfill_jobs ADD COLUMN ended_at_ms INTEGER; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
