-- ABOUTME: activity_route_tracks admits 'unavailable' — a route read that failed, stored briefly and never read as no_gps
-- ABOUTME: Drops the no_gps rows an unproven read stored, so each is read again instead of hiding a map for a day

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A route read whose detail answer failed, timed out or carried no stream
-- set proves nothing about the activity: it is now stored as 'unavailable'
-- with a short expires_at, so a Home page that asks again within minutes is
-- answered without reaching a scraper that has just failed, and the list
-- keeps has_gps true. Only a stream set without coordinates is 'no_gps'.
--
-- SQLite has no ALTER for a CHECK constraint: the table is copied under the
-- widened constraint, the original dropped, the copy renamed and the index
-- recreated. sqlx runs the migration in one transaction and no other table
-- references this one by foreign key, so no PRAGMA is needed.
--
-- The no_gps rows that carry an expiry were stored from a detail read with
-- no stream set, which is exactly the failed read this migration stops
-- calling no_gps; they are not copied, so each is read again.
CREATE TABLE IF NOT EXISTS activity_route_tracks_new (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    activity_id TEXT NOT NULL,
    source TEXT NOT NULL CHECK (source IN ('summary_polyline', 'streams')),
    track_json TEXT,
    unavailable_reason TEXT CHECK (unavailable_reason IN ('no_gps', 'too_short', 'unavailable')),
    created_at TEXT NOT NULL,
    expires_at TEXT,
    PRIMARY KEY (tenant_id, user_id, provider, activity_id),
    CHECK ((track_json IS NULL) <> (unavailable_reason IS NULL))
);
INSERT INTO activity_route_tracks_new (tenant_id, user_id, provider, activity_id, source,
    track_json, unavailable_reason, created_at, expires_at)
  SELECT tenant_id, user_id, provider, activity_id, source, track_json, unavailable_reason,
    created_at, expires_at
  FROM activity_route_tracks
  WHERE NOT (unavailable_reason = 'no_gps' AND expires_at IS NOT NULL);
DROP TABLE IF EXISTS activity_route_tracks;
ALTER TABLE activity_route_tracks_new RENAME TO activity_route_tracks;

CREATE INDEX IF NOT EXISTS idx_activity_route_tracks_user_tenant
    ON activity_route_tracks(user_id, tenant_id);
