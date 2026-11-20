-- ABOUTME: The day a history walk's paused wellness copy resumes from, on its job row (SQLite)
-- ABOUTME: Additive only: one nullable column on activity_backfill_jobs; nothing is renamed or dropped (carnet#1661)
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A Garmin or COROS history walk copies the year of wellness before the
-- athlete's push grant once its activities are in. Every day it reads is one
-- read of the athlete's session, counted in their session-read caps
-- (carnet#1661). When a cap refuses a day, the copy pauses until the refusing
-- window ends, and the day it stopped at is kept here (ISO `YYYY-MM-DD`), so
-- the next run copies from it rather than from the window's start. NULL
-- copies the whole window; the walk's end and a new ask of the row clear it.
ALTER TABLE activity_backfill_jobs ADD COLUMN wellness_resume_day TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
