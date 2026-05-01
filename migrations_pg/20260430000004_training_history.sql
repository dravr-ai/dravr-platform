-- ABOUTME: Phase 2 Endurance — daily training-history rollup table backing GET /api/v1/endurance/history
-- ABOUTME: One row per (tenant_id, user_id, date) carrying CTL/ATL/TSB/ACWR/monotony/strain/ramp_rate/daily_load
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

CREATE TABLE IF NOT EXISTS training_history (
    tenant_id UUID NOT NULL,
    user_id UUID NOT NULL,
    date DATE NOT NULL,
    ctl DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    atl DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    tsb DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    acwr DOUBLE PRECISION,
    monotony DOUBLE PRECISION,
    strain DOUBLE PRECISION,
    ramp_rate DOUBLE PRECISION,
    daily_load DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    computed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, user_id, date)
);

CREATE INDEX IF NOT EXISTS idx_training_history_tenant_user_date
    ON training_history(tenant_id, user_id, date DESC);
