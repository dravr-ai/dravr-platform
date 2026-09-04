-- ABOUTME: Training catalogue — the eight catalogue columns of a user-authored workout template
-- ABOUTME: purpose, sport variants, evidence tier and caveat, params, progression, phase fit, evidence refs
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- A stored row carried only the Phase 5 shape (sport, duration, intensity
-- distribution, structure, target zones); the catalogue layer was derived
-- from intensity_distribution on every read. The columns below persist what
-- the kernel's WorkoutTemplate carries, so a user-authored template keeps the
-- purpose, ranges and fit it was written with. The text columns hold the
-- vocabulary's snake_case names; the JSONB columns hold the serde JSON of
-- WorkoutParams, Progression, PhaseFit and the two lists.

ALTER TABLE workout_templates ADD COLUMN IF NOT EXISTS purpose TEXT NOT NULL DEFAULT 'endurance';
ALTER TABLE workout_templates ADD COLUMN IF NOT EXISTS sport_variants_json JSONB NOT NULL DEFAULT '[]'::jsonb;
ALTER TABLE workout_templates ADD COLUMN IF NOT EXISTS evidence_tier TEXT NOT NULL DEFAULT 'coach_judgement';
ALTER TABLE workout_templates ADD COLUMN IF NOT EXISTS caveat TEXT;
ALTER TABLE workout_templates ADD COLUMN IF NOT EXISTS params_json JSONB NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE workout_templates ADD COLUMN IF NOT EXISTS progression_json JSONB NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE workout_templates ADD COLUMN IF NOT EXISTS fit_json JSONB NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE workout_templates ADD COLUMN IF NOT EXISTS evidence_refs_json JSONB NOT NULL DEFAULT '[]'::jsonb;

-- Existing rows get the purpose the read path used to derive
-- (WorkoutTemplate::inline_defaults), so nothing a coach saw changes.
UPDATE workout_templates SET purpose = CASE intensity_distribution
    WHEN '"recovery"' THEN 'recovery'
    WHEN '"threshold"' THEN 'threshold'
    WHEN '"vo2max"' THEN 'vo2max_long'
    WHEN '"pyramid"' THEN 'tempo'
    ELSE 'endurance'
END;

CREATE INDEX IF NOT EXISTS idx_workout_templates_purpose
    ON workout_templates(purpose);
