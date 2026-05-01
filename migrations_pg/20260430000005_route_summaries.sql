-- ABOUTME: Phase 3 Endurance — route_summaries cache table for parsed GPX terrain analysis
-- ABOUTME: One row per (tenant_id, user_id, activity_id) carrying terrain JSON + climbs JSON keyed by gpx_hash
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

CREATE TABLE IF NOT EXISTS route_summaries (
    tenant_id UUID NOT NULL,
    user_id UUID NOT NULL,
    activity_id TEXT NOT NULL,
    gpx_hash TEXT NOT NULL,
    terrain_summary_json JSONB NOT NULL,
    climbs_json JSONB NOT NULL,
    computed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, user_id, activity_id)
);

CREATE INDEX IF NOT EXISTS idx_route_summaries_tenant_user
    ON route_summaries(tenant_id, user_id);
CREATE INDEX IF NOT EXISTS idx_route_summaries_hash
    ON route_summaries(gpx_hash);
