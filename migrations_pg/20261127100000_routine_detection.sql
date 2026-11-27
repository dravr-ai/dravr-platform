-- ABOUTME: The athlete's per-coach consent to routine detection: their coach told of a break longer than their own usual ones (PostgreSQL)
-- ABOUTME: Off by default (profiling, Law 25 art. 8.1); the stamp records every answer, "not now" included, which clears the card

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Routines P3b, routine detection (carnet#1792). See the SQLite twin for
-- what each column means.
ALTER TABLE coaching_group_members ADD COLUMN IF NOT EXISTS coach_routine_detection BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE coaching_group_members ADD COLUMN IF NOT EXISTS coach_routine_detection_at TIMESTAMPTZ;
