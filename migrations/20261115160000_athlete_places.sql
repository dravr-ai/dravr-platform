-- ABOUTME: athlete_places — the places an athlete trains from, clustered from trimmed activity starts
-- ABOUTME: athlete_place_runs — what the last clustering of a user saw, so the places worker finds whom to recompute
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- A place is a cluster of an athlete's trimmed activity starts (carnet#1484,
-- slice 6). Its id is a uuid a re-clustering hands on to the one cluster that
-- inherits it, so a name the athlete gave survives new activities. A centroid
-- is built only from privacy-trimmed starts, and is never served by any tool
-- or route: places are served as {id, name}. The athlete's own data export
-- carries it.
--
-- name_source says where a name came from: only an athlete-given name is
-- served over external transports; a city or segment name is first-party only.
--
-- A user's runs row records the cached row count and newest sync the last
-- clustering saw, so a single statement over column aggregates finds the users
-- whose places are due, with no hook in the ingest, erasure or purge paths.
-- Ids are TEXT, like cached_activities; timestamps are RFC 3339 text.

CREATE TABLE IF NOT EXISTS athlete_places (
    id TEXT NOT NULL PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    centroid_lat REAL NOT NULL,
    centroid_lon REAL NOT NULL,
    radius_m REAL NOT NULL,
    activity_count INTEGER NOT NULL,
    name TEXT,
    name_source TEXT CHECK (name_source IN ('athlete', 'city', 'segment')),
    version INTEGER NOT NULL,
    computed_at TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_athlete_places_user ON athlete_places (tenant_id, user_id);

CREATE TABLE IF NOT EXISTS athlete_place_runs (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    rows_seen INTEGER NOT NULL,
    newest_synced_at TEXT,
    version INTEGER NOT NULL,
    computed_at TEXT NOT NULL,
    PRIMARY KEY (tenant_id, user_id)
);
