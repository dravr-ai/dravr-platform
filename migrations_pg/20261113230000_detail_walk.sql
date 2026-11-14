-- ABOUTME: The sport a detail walk reads, beside the window and progress columns it reuses from the history walk (Postgres)
-- ABOUTME: Additive only: one nullable column on activity_backfill_jobs; nothing is renamed or dropped (carnet#1483)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the matching SQLite migration for the columns a detail walk reuses.
--   walk_sport  the sport type the walk reads, NULL for every sport
ALTER TABLE activity_backfill_jobs ADD COLUMN IF NOT EXISTS walk_sport TEXT;
