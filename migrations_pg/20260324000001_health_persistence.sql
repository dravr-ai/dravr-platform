-- ABOUTME: PostgreSQL health persistence schema (data sources, sleep, recovery, health, time-series)
-- ABOUTME: Mirrors migrations/20260324000001_health_persistence.sql with PG-native types
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Data source tracking (device/provider identity for deduplication)
CREATE TABLE IF NOT EXISTS data_sources (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    device_model TEXT,
    software_version TEXT,
    source TEXT,
    device_type TEXT NOT NULL DEFAULT 'unknown',
    original_source_name TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, tenant_id, provider, device_model, source)
);
CREATE INDEX IF NOT EXISTS idx_data_sources_user_tenant ON data_sources(user_id, tenant_id);
CREATE INDEX IF NOT EXISTS idx_data_sources_provider ON data_sources(provider);

-- Sleep sessions (persisted from providers via dravr-equilibre)
CREATE TABLE IF NOT EXISTS sleep_sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    data_source_id TEXT REFERENCES data_sources(id),
    synced_at TIMESTAMPTZ NOT NULL,
    start_time TIMESTAMPTZ NOT NULL,
    end_time TIMESTAMPTZ NOT NULL,
    time_in_bed BIGINT NOT NULL,
    total_sleep_time BIGINT NOT NULL,
    sleep_efficiency DOUBLE PRECISION NOT NULL,
    sleep_score DOUBLE PRECISION,
    stages_json TEXT NOT NULL DEFAULT '[]',
    hrv_during_sleep DOUBLE PRECISION,
    respiratory_rate DOUBLE PRECISION,
    temperature_variation DOUBLE PRECISION,
    wake_count INTEGER,
    sleep_onset_latency INTEGER,
    is_nap INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, tenant_id, provider, start_time)
);
CREATE INDEX IF NOT EXISTS idx_sleep_sessions_user_tenant ON sleep_sessions(user_id, tenant_id);
CREATE INDEX IF NOT EXISTS idx_sleep_sessions_dates ON sleep_sessions(start_time, end_time);

-- Recovery metrics (daily recovery/readiness scores)
CREATE TABLE IF NOT EXISTS recovery_metrics (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    data_source_id TEXT REFERENCES data_sources(id),
    synced_at TIMESTAMPTZ NOT NULL,
    date DATE NOT NULL,
    recovery_score DOUBLE PRECISION,
    readiness_score DOUBLE PRECISION,
    hrv_status TEXT,
    sleep_score DOUBLE PRECISION,
    stress_level DOUBLE PRECISION,
    training_load DOUBLE PRECISION,
    resting_heart_rate INTEGER,
    body_temperature DOUBLE PRECISION,
    resting_respiratory_rate DOUBLE PRECISION,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, tenant_id, provider, date)
);
CREATE INDEX IF NOT EXISTS idx_recovery_metrics_user_tenant ON recovery_metrics(user_id, tenant_id);
CREATE INDEX IF NOT EXISTS idx_recovery_metrics_date ON recovery_metrics(date);

-- Health snapshots (body composition and vitals)
CREATE TABLE IF NOT EXISTS health_snapshots (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    data_source_id TEXT REFERENCES data_sources(id),
    synced_at TIMESTAMPTZ NOT NULL,
    date DATE NOT NULL,
    weight DOUBLE PRECISION,
    body_fat_percentage DOUBLE PRECISION,
    muscle_mass DOUBLE PRECISION,
    bone_mass DOUBLE PRECISION,
    body_water_percentage DOUBLE PRECISION,
    bmr INTEGER,
    bp_systolic INTEGER,
    bp_diastolic INTEGER,
    blood_glucose DOUBLE PRECISION,
    vo2_max DOUBLE PRECISION,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, tenant_id, provider, date)
);
CREATE INDEX IF NOT EXISTS idx_health_snapshots_user_tenant ON health_snapshots(user_id, tenant_id);
CREATE INDEX IF NOT EXISTS idx_health_snapshots_date ON health_snapshots(date);

-- Time-series data points (unified storage for all metric types)
CREATE TABLE IF NOT EXISTS data_point_series (
    id TEXT PRIMARY KEY,
    data_source_id TEXT NOT NULL REFERENCES data_sources(id) ON DELETE CASCADE,
    series_type_id BIGINT NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL,
    zone_offset TEXT,
    value DOUBLE PRECISION NOT NULL,
    UNIQUE(data_source_id, series_type_id, recorded_at)
);
CREATE INDEX IF NOT EXISTS idx_data_point_series_source_type ON data_point_series(data_source_id, series_type_id);
CREATE INDEX IF NOT EXISTS idx_data_point_series_recorded ON data_point_series(recorded_at);

-- Time-series daily archives (aggregated rollups for data lifecycle)
CREATE TABLE IF NOT EXISTS data_point_series_archive (
    id TEXT PRIMARY KEY,
    data_source_id TEXT NOT NULL REFERENCES data_sources(id) ON DELETE CASCADE,
    series_type_id BIGINT NOT NULL,
    bucket_start_at TIMESTAMPTZ NOT NULL,
    aggregation_type TEXT NOT NULL,
    value DOUBLE PRECISION NOT NULL,
    sample_count BIGINT NOT NULL,
    UNIQUE(data_source_id, series_type_id, bucket_start_at, aggregation_type)
);
CREATE INDEX IF NOT EXISTS idx_archive_source_type ON data_point_series_archive(data_source_id, series_type_id);
