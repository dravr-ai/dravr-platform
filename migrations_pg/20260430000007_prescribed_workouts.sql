-- ABOUTME: Phase 5 Endurance — prescribed_workouts audit trail for coach-driven push to providers
-- ABOUTME: One row per (tenant_id, user_id, prescribed_for_date) carrying provider event id + template slug
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

CREATE TABLE IF NOT EXISTS prescribed_workouts (
    id UUID PRIMARY KEY,
    tenant_id UUID NOT NULL,
    user_id UUID NOT NULL,
    coach_id TEXT,
    template_slug TEXT NOT NULL,
    sport TEXT NOT NULL,
    prescribed_for_date DATE NOT NULL,
    provider TEXT NOT NULL,
    provider_event_id TEXT,
    payload_json JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'pushed',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_prescribed_workouts_tenant_user
    ON prescribed_workouts(tenant_id, user_id);
CREATE INDEX IF NOT EXISTS idx_prescribed_workouts_date
    ON prescribed_workouts(tenant_id, user_id, prescribed_for_date DESC);
