-- ABOUTME: Drops the intervals.icu route tracks drawn from streams, so each is read again with its real longitudes
-- ABOUTME: Those tracks paired consecutive latitudes as (lat, lon) and drew every route as a diagonal at (lat, lat)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- intervals.icu sends a latlng stream's latitudes in `data` and its
-- longitudes in `data2`. Every track read from those streams before the
-- provider read `data2` holds latitudes in both columns, and a drawn track
-- is stored without an expiry, so none of them would ever be read again.
-- Deleting them makes the next Home request read each route afresh. Rows
-- that settled no_gps or too_short came from the same streams and are
-- dropped too: too_short was measured on the wrong geometry.
DELETE FROM activity_route_tracks
WHERE provider = 'intervals_icu' AND source = 'streams';
