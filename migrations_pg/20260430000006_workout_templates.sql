-- ABOUTME: Phase 5 Endurance — workout_templates table for Endurance prescription library
-- ABOUTME: Compiled-in cornerstones live in workout_templates/*.toml; user-authored rows persist here
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

CREATE TABLE IF NOT EXISTS workout_templates (
    id UUID PRIMARY KEY,
    tenant_id UUID,
    user_id UUID,
    slug TEXT NOT NULL,
    name TEXT NOT NULL,
    sport TEXT NOT NULL,
    duration_minutes INTEGER NOT NULL,
    intensity_distribution TEXT NOT NULL,
    structure_json JSONB NOT NULL,
    target_zones_json JSONB NOT NULL,
    is_compiled_in BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_workout_templates_slug
    ON workout_templates(tenant_id, user_id, slug);
CREATE INDEX IF NOT EXISTS idx_workout_templates_sport
    ON workout_templates(sport);
