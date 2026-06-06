-- ABOUTME: PostgreSQL activity cache schema (provider-agnostic activity persistence)
-- ABOUTME: Mirrors migrations/20260606000001_activity_cache.sql with PG-native types
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

CREATE TABLE IF NOT EXISTS cached_activities (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    activity_id TEXT NOT NULL,
    sport_type TEXT,
    start_date TIMESTAMPTZ NOT NULL,
    synced_at TIMESTAMPTZ NOT NULL,
    data_json TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, tenant_id, provider, activity_id)
);
CREATE INDEX IF NOT EXISTS idx_cached_activities_user_tenant ON cached_activities(user_id, tenant_id);
CREATE INDEX IF NOT EXISTS idx_cached_activities_start_date ON cached_activities(start_date);
