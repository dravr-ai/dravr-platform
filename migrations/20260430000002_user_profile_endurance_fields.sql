-- ABOUTME: Phase 1 Endurance — user physiological profile table with Endurance fields
-- ABOUTME: Backs UserPhysiologicalProfile + ftp_watts/threshold_pace/hr_zones/power_zones for latest.json + dossier.json
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

CREATE TABLE IF NOT EXISTS user_physiological_profiles (
    user_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    vo2_max REAL,
    resting_hr INTEGER,
    max_hr INTEGER,
    lactate_threshold_percentage REAL,
    age INTEGER,
    weight REAL,
    fitness_level TEXT NOT NULL DEFAULT 'recreational',
    primary_sport TEXT NOT NULL DEFAULT 'running',
    training_experience_years INTEGER,
    ftp_watts INTEGER,
    threshold_pace_sec_per_km REAL,
    hr_zones_json TEXT,
    power_zones_json TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (tenant_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_user_physio_tenant ON user_physiological_profiles(tenant_id);
CREATE INDEX IF NOT EXISTS idx_user_physio_tenant_user ON user_physiological_profiles(tenant_id, user_id);
