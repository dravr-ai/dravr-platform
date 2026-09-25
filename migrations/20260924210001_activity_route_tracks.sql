-- ABOUTME: activity_route_tracks — one activity's privacy-trimmed, simplified route, read once per activity
-- ABOUTME: A drawn track as JSON, or the reason there is none, keyed by (tenant_id, user_id, provider, activity_id)
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A completed activity's route does not change, so the Home page reads each
-- activity's geometry once — from the provider's route overview when the
-- cached activity carries one, from its recorded streams otherwise — and
-- keeps it here. The geometry is trimmed at its endpoints before it is
-- stored, never after. An activity with no drawable route (indoor, trainer,
-- a ride that never leaves its doorstep) is stored with its reason instead,
-- so it costs one provider read too, not one per page load.
--
-- Rows name their provider so the provider-disconnect purge deletes them with
-- the rest of that provider's data. Ids are TEXT, like cached_activities.

CREATE TABLE IF NOT EXISTS activity_route_tracks (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    activity_id TEXT NOT NULL,
    source TEXT NOT NULL CHECK (source IN ('summary_polyline', 'streams')),
    track_json TEXT,
    unavailable_reason TEXT CHECK (unavailable_reason IN ('no_gps', 'too_short')),
    created_at TEXT NOT NULL,
    PRIMARY KEY (tenant_id, user_id, provider, activity_id),
    CHECK ((track_json IS NULL) <> (unavailable_reason IS NULL))
);

CREATE INDEX IF NOT EXISTS idx_activity_route_tracks_user_tenant
    ON activity_route_tracks(user_id, tenant_id);
