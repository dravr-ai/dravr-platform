-- ABOUTME: athlete_place_runs.newest_track_at_seen — the newest drawn route track the last successful places recompute saw (PostgreSQL version)
-- ABOUTME: A drawn track stored after it makes the athlete due, so a ride whose track arrives on a detail read gets its place
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- A drawn track in activity_route_tracks gives a ride the trimmed start a
-- place forms from, and it arrives on a read of the ride's detail or route
-- (carnet#1679) that changes neither the athlete's cached row count nor their
-- newest sync. A recompute records the newest created_at among the athlete's
-- drawn tracks, read before its rows; a drawn track stored after it keeps the
-- athlete due. A failed recompute leaves the column alone, so they stay due.
-- A run written before this column has none, and makes an athlete holding any
-- drawn track due once. An answer that a ride has no track forms no place and
-- is not counted.

ALTER TABLE athlete_place_runs ADD COLUMN IF NOT EXISTS newest_track_at_seen TIMESTAMPTZ;
