-- ABOUTME: Adds the daily_activity table and sleep_sessions.avg_spo2 (PostgreSQL)
-- ABOUTME: Homes for a day's steps and energy and a night's pulse-ox, which had no readable store (carnet#1281)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A day's steps, active and total energy and intensity minutes had no table:
-- a Garmin export wrote them only as time-series points no reader reads, and
-- the Garmin push and capture dropped them. One row per user, tenant,
-- provider and calendar date, keyed and governed like recovery_metrics and
-- health_snapshots (Athlete Data Consolidation design, S0). Additive; no
-- writer or reader exists yet, so the table starts empty.
CREATE TABLE IF NOT EXISTS daily_activity (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    data_source_id TEXT REFERENCES data_sources(id),
    synced_at TIMESTAMPTZ NOT NULL,
    date DATE NOT NULL,
    steps INTEGER,
    active_kcal DOUBLE PRECISION,
    total_kcal DOUBLE PRECISION,
    intensity_minutes INTEGER,
    deleted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, tenant_id, provider, date)
);
CREATE INDEX IF NOT EXISTS idx_daily_activity_user_tenant ON daily_activity(user_id, tenant_id);
CREATE INDEX IF NOT EXISTS idx_daily_activity_date ON daily_activity(date);

-- The night's average pulse-ox, which the Garmin push carries on sleep and
-- had nowhere to go (the day's SpO2 already fills recovery_metrics.spo2).
ALTER TABLE sleep_sessions ADD COLUMN IF NOT EXISTS avg_spo2 DOUBLE PRECISION;
