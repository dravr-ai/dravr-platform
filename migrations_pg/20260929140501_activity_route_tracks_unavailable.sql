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
-- PostgreSQL named the inline column constraint
-- activity_route_tracks_unavailable_reason_check; it is replaced under the
-- same name so the table keeps exactly one reason CHECK.
--
-- The no_gps rows that carry an expiry were stored from a detail read with
-- no stream set, which is exactly the failed read this migration stops
-- calling no_gps; they are deleted, so each is read again.
DELETE FROM activity_route_tracks
WHERE unavailable_reason = 'no_gps' AND expires_at IS NOT NULL;
ALTER TABLE activity_route_tracks
    DROP CONSTRAINT IF EXISTS activity_route_tracks_unavailable_reason_check;
ALTER TABLE activity_route_tracks
    ADD CONSTRAINT activity_route_tracks_unavailable_reason_check
    CHECK (unavailable_reason IN ('no_gps', 'too_short', 'unavailable'));
