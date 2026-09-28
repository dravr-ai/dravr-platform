-- ABOUTME: activity_route_tracks.expires_at — when a stored no-GPS answer that proves nothing is read again (PostgreSQL version)
-- ABOUTME: NULL for an answer that stands; the no_gps rows stored before the column existed are read once more

-- A detail read whose answer carries no stream set at all — a mirror scrape
-- that found no map on the page, a streams request that failed and was served
-- without — does not show the activity recorded no GPS, so its `no_gps`
-- answer is stored with the instant it is read again. Past it, the Home list
-- treats the activity's route as not read and the route read asks the
-- provider once more. Every other answer stands and carries NULL.
--
-- The `no_gps` rows stored before this column were stored from either kind of
-- answer, so they are given their own `created_at`: expired, and read once
-- more to find out which they were.
ALTER TABLE activity_route_tracks ADD COLUMN IF NOT EXISTS expires_at TIMESTAMPTZ;

UPDATE activity_route_tracks
SET expires_at = created_at
WHERE unavailable_reason = 'no_gps' AND expires_at IS NULL;
