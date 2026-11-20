-- ABOUTME: The day a history walk's paused wellness copy resumes from, on its job row (Postgres)
-- ABOUTME: Additive only: one nullable column on activity_backfill_jobs; nothing is renamed or dropped (carnet#1661)
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the matching SQLite migration. The day a paused wellness copy resumes
-- from (ISO `YYYY-MM-DD`); NULL copies the whole window.
ALTER TABLE activity_backfill_jobs ADD COLUMN IF NOT EXISTS wellness_resume_day TEXT;
