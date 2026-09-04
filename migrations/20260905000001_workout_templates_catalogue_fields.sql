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
-- vocabulary's snake_case names; the *_json columns hold the serde JSON of
-- WorkoutParams, Progression, PhaseFit and the two lists.

ALTER TABLE workout_templates ADD COLUMN purpose TEXT NOT NULL DEFAULT 'endurance';  -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE workout_templates ADD COLUMN sport_variants_json TEXT NOT NULL DEFAULT '[]';  -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE workout_templates ADD COLUMN evidence_tier TEXT NOT NULL DEFAULT 'coach_judgement';  -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE workout_templates ADD COLUMN caveat TEXT;  -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE workout_templates ADD COLUMN params_json TEXT NOT NULL DEFAULT '{}';  -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE workout_templates ADD COLUMN progression_json TEXT NOT NULL DEFAULT '{}';  -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE workout_templates ADD COLUMN fit_json TEXT NOT NULL DEFAULT '{}';  -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE workout_templates ADD COLUMN evidence_refs_json TEXT NOT NULL DEFAULT '[]';  -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

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
