-- ABOUTME: Usage counters table for rate limiting and quota enforcement
-- ABOUTME: Stores per-tenant per-user rolling counters with time-bucketed keys
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

CREATE TABLE IF NOT EXISTS usage_counters (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    counter_key TEXT NOT NULL,
    period TEXT NOT NULL,
    value INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (tenant_id, user_id, counter_key, period)
);

CREATE INDEX IF NOT EXISTS idx_usage_counters_tenant_user ON usage_counters(tenant_id, user_id);
CREATE INDEX IF NOT EXISTS idx_usage_counters_period ON usage_counters(period);
