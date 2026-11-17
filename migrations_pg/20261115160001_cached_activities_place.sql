-- ABOUTME: cached_activities gains the place an activity's start belongs to (PostgreSQL version)
-- ABOUTME: One nullable column no older binary selects, so a rollback deploy keeps working
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- carnet#1484, slice 6. place_id is the athlete_places row the row's
-- trimmed start belongs to, written only by the places service; the list
-- sync never writes it.

ALTER TABLE cached_activities ADD COLUMN IF NOT EXISTS place_id TEXT;

-- A user's rows by place.
CREATE INDEX IF NOT EXISTS idx_cached_activities_place
    ON cached_activities (tenant_id, user_id, place_id);
