-- Activity cache: provider-agnostic persistence of fetched activities
-- so chat turns serve cached activities (stale-while-revalidate) instead of
-- re-calling the provider every time. Applies to all providers (Strava, WHOOP,
-- Garmin/sciotte, Fitbit, Intervals.icu, ...). Mirrors the health-persistence
-- pattern: lean queryable columns + a full serialized Activity in data_json.
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

CREATE TABLE IF NOT EXISTS cached_activities (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    activity_id TEXT NOT NULL,
    sport_type TEXT,
    start_date TEXT NOT NULL,
    synced_at TEXT NOT NULL,
    data_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(user_id, tenant_id, provider, activity_id)
);
CREATE INDEX IF NOT EXISTS idx_cached_activities_user_tenant ON cached_activities(user_id, tenant_id);
CREATE INDEX IF NOT EXISTS idx_cached_activities_start_date ON cached_activities(start_date);
