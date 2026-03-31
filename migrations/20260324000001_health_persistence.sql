-- Health persistence schema: data sources, sleep, recovery, health, and time-series
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Data source tracking (device/provider identity for deduplication)
CREATE TABLE IF NOT EXISTS data_sources (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    device_model TEXT,
    software_version TEXT,
    source TEXT,
    device_type TEXT NOT NULL DEFAULT 'unknown',
    original_source_name TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(user_id, tenant_id, provider, device_model, source)
);
CREATE INDEX IF NOT EXISTS idx_data_sources_user_tenant ON data_sources(user_id, tenant_id);
CREATE INDEX IF NOT EXISTS idx_data_sources_provider ON data_sources(provider);

-- Sleep sessions (persisted from providers via dravr-equilibre)
CREATE TABLE IF NOT EXISTS sleep_sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    data_source_id TEXT REFERENCES data_sources(id),
    synced_at TEXT NOT NULL,
    start_time TEXT NOT NULL,
    end_time TEXT NOT NULL,
    time_in_bed INTEGER NOT NULL,
    total_sleep_time INTEGER NOT NULL,
    sleep_efficiency REAL NOT NULL,
    sleep_score REAL,
    stages_json TEXT NOT NULL DEFAULT '[]',
    hrv_during_sleep REAL,
    respiratory_rate REAL,
    temperature_variation REAL,
    wake_count INTEGER,
    sleep_onset_latency INTEGER,
    is_nap INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(user_id, tenant_id, provider, start_time)
);
CREATE INDEX IF NOT EXISTS idx_sleep_sessions_user_tenant ON sleep_sessions(user_id, tenant_id);
CREATE INDEX IF NOT EXISTS idx_sleep_sessions_dates ON sleep_sessions(start_time, end_time);

-- Recovery metrics (daily recovery/readiness scores)
CREATE TABLE IF NOT EXISTS recovery_metrics (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    data_source_id TEXT REFERENCES data_sources(id),
    synced_at TEXT NOT NULL,
    date TEXT NOT NULL,
    recovery_score REAL,
    readiness_score REAL,
    hrv_status TEXT,
    sleep_score REAL,
    stress_level REAL,
    training_load REAL,
    resting_heart_rate INTEGER,
    body_temperature REAL,
    resting_respiratory_rate REAL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(user_id, tenant_id, provider, date)
);
CREATE INDEX IF NOT EXISTS idx_recovery_metrics_user_tenant ON recovery_metrics(user_id, tenant_id);
CREATE INDEX IF NOT EXISTS idx_recovery_metrics_date ON recovery_metrics(date);

-- Health snapshots (body composition and vitals)
CREATE TABLE IF NOT EXISTS health_snapshots (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    data_source_id TEXT REFERENCES data_sources(id),
    synced_at TEXT NOT NULL,
    date TEXT NOT NULL,
    weight REAL,
    body_fat_percentage REAL,
    muscle_mass REAL,
    bone_mass REAL,
    body_water_percentage REAL,
    bmr INTEGER,
    bp_systolic INTEGER,
    bp_diastolic INTEGER,
    blood_glucose REAL,
    vo2_max REAL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(user_id, tenant_id, provider, date)
);
CREATE INDEX IF NOT EXISTS idx_health_snapshots_user_tenant ON health_snapshots(user_id, tenant_id);
CREATE INDEX IF NOT EXISTS idx_health_snapshots_date ON health_snapshots(date);

-- Time-series data points (unified storage for all metric types)
CREATE TABLE IF NOT EXISTS data_point_series (
    id TEXT PRIMARY KEY,
    data_source_id TEXT NOT NULL REFERENCES data_sources(id) ON DELETE CASCADE,
    series_type_id INTEGER NOT NULL,
    recorded_at TEXT NOT NULL,
    zone_offset TEXT,
    value REAL NOT NULL,
    UNIQUE(data_source_id, series_type_id, recorded_at)
);
CREATE INDEX IF NOT EXISTS idx_data_point_series_source_type ON data_point_series(data_source_id, series_type_id);
CREATE INDEX IF NOT EXISTS idx_data_point_series_recorded ON data_point_series(recorded_at);

-- Time-series daily archives (aggregated rollups for data lifecycle)
CREATE TABLE IF NOT EXISTS data_point_series_archive (
    id TEXT PRIMARY KEY,
    data_source_id TEXT NOT NULL REFERENCES data_sources(id) ON DELETE CASCADE,
    series_type_id INTEGER NOT NULL,
    bucket_start_at TEXT NOT NULL,
    aggregation_type TEXT NOT NULL,
    value REAL NOT NULL,
    sample_count INTEGER NOT NULL,
    UNIQUE(data_source_id, series_type_id, bucket_start_at, aggregation_type)
);
CREATE INDEX IF NOT EXISTS idx_archive_source_type ON data_point_series_archive(data_source_id, series_type_id);
