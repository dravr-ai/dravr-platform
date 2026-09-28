-- ABOUTME: form_ctl on training_history — the CTL a day's form is a share of (PostgreSQL)
-- ABOUTME: Clears every row stored under same-day form; the rollup recomputes them from the activity cache

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the matching SQLite migration for the full rationale (carnet#601).
DELETE FROM training_history;
ALTER TABLE training_history ADD COLUMN IF NOT EXISTS form_ctl DOUBLE PRECISION;
