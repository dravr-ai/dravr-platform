-- ABOUTME: Phase 5 Endurance — workout_templates table for Endurance prescription library
-- ABOUTME: Compiled-in cornerstones live in workout_templates/*.toml; user-authored rows persist here
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

CREATE TABLE IF NOT EXISTS workout_templates (
    id TEXT PRIMARY KEY,
    tenant_id TEXT,
    user_id TEXT,
    slug TEXT NOT NULL,
    name TEXT NOT NULL,
    sport TEXT NOT NULL,
    duration_minutes INTEGER NOT NULL,
    intensity_distribution TEXT NOT NULL,
    structure_json TEXT NOT NULL,
    target_zones_json TEXT NOT NULL,
    is_compiled_in INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_workout_templates_slug
    ON workout_templates(tenant_id, user_id, slug);
CREATE INDEX IF NOT EXISTS idx_workout_templates_sport
    ON workout_templates(sport);
