-- ABOUTME: The sport a detail walk reads, its window's top and when it ended, beside the columns it reuses (Postgres)
-- ABOUTME: Additive only: seven nullable columns on activity_backfill_jobs; nothing is renamed or dropped (carnet#1483)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the matching SQLite migration for the columns a detail walk reuses.
--   walk_sport     the sport type the walk reads, NULL for every sport
--   window_top_ts  the window's top as the ask that recorded the row named it
--   ended_at_ms    when a detail walk ended walked, NULL while owed or after
--                  any other end
--   tried_floor_ts, tried_top_ts, tried_asked_ms, tried_ended_ms
--                  the range an earlier walk that ended walked within a week
--                  tried, when it was asked, and when the oldest walk whose
--                  tries the range carries ended
ALTER TABLE activity_backfill_jobs ADD COLUMN IF NOT EXISTS walk_sport TEXT;
ALTER TABLE activity_backfill_jobs ADD COLUMN IF NOT EXISTS window_top_ts BIGINT;
ALTER TABLE activity_backfill_jobs ADD COLUMN IF NOT EXISTS ended_at_ms BIGINT;
ALTER TABLE activity_backfill_jobs ADD COLUMN IF NOT EXISTS tried_floor_ts BIGINT;
ALTER TABLE activity_backfill_jobs ADD COLUMN IF NOT EXISTS tried_top_ts BIGINT;
ALTER TABLE activity_backfill_jobs ADD COLUMN IF NOT EXISTS tried_asked_ms BIGINT;
ALTER TABLE activity_backfill_jobs ADD COLUMN IF NOT EXISTS tried_ended_ms BIGINT;
