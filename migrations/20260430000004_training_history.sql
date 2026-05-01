-- ABOUTME: Phase 2 Endurance — daily training-history rollup table backing GET /api/v1/endurance/history
-- ABOUTME: One row per (tenant_id, user_id, date) carrying CTL/ATL/TSB/ACWR/monotony/strain/ramp_rate/daily_load
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

CREATE TABLE IF NOT EXISTS training_history (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    date TEXT NOT NULL,
    ctl REAL NOT NULL DEFAULT 0.0,
    atl REAL NOT NULL DEFAULT 0.0,
    tsb REAL NOT NULL DEFAULT 0.0,
    acwr REAL,
    monotony REAL,
    strain REAL,
    ramp_rate REAL,
    daily_load REAL NOT NULL DEFAULT 0.0,
    computed_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (tenant_id, user_id, date)
);

CREATE INDEX IF NOT EXISTS idx_training_history_tenant_user_date
    ON training_history(tenant_id, user_id, date DESC);
