-- ABOUTME: Phase 1 Endurance — user physiological profile table with Endurance fields
-- ABOUTME: Backs UserPhysiologicalProfile + ftp_watts/threshold_pace/hr_zones/power_zones for latest.json + dossier.json
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

CREATE TABLE IF NOT EXISTS user_physiological_profiles (
    user_id UUID NOT NULL,
    tenant_id UUID NOT NULL,
    vo2_max DOUBLE PRECISION,
    resting_hr INTEGER,
    max_hr INTEGER,
    lactate_threshold_percentage DOUBLE PRECISION,
    age INTEGER,
    weight DOUBLE PRECISION,
    fitness_level TEXT NOT NULL DEFAULT 'recreational',
    primary_sport TEXT NOT NULL DEFAULT 'running',
    training_experience_years INTEGER,
    ftp_watts INTEGER,
    threshold_pace_sec_per_km DOUBLE PRECISION,
    hr_zones_json JSONB,
    power_zones_json JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_user_physio_tenant ON user_physiological_profiles(tenant_id);
CREATE INDEX IF NOT EXISTS idx_user_physio_tenant_user ON user_physiological_profiles(tenant_id, user_id);
