-- ABOUTME: The sport a detail walk reads, its window's top and when it ended, beside the columns it reuses (Postgres)
-- ABOUTME: Additive only: three nullable columns on activity_backfill_jobs; nothing is renamed or dropped (carnet#1483)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the matching SQLite migration for the columns a detail walk reuses.
--   walk_sport     the sport type the walk reads, NULL for every sport
--   window_top_ts  the window's top as the ask that recorded the row named it
--   ended_at_ms    when a detail walk ended walked, NULL while owed or after
--                  any other end
ALTER TABLE activity_backfill_jobs ADD COLUMN IF NOT EXISTS walk_sport TEXT;
ALTER TABLE activity_backfill_jobs ADD COLUMN IF NOT EXISTS window_top_ts BIGINT;
ALTER TABLE activity_backfill_jobs ADD COLUMN IF NOT EXISTS ended_at_ms BIGINT;
