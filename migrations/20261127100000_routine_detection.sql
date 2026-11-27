-- ABOUTME: The athlete's per-coach consent to routine detection: their coach told of a break longer than their own usual ones
-- ABOUTME: Off by default (profiling, Law 25 art. 8.1); the stamp records every answer, "not now" included, which clears the card

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Routines P3b, routine detection (carnet#1792).
--
--   coaching_group_members.coach_routine_detection
--       the group's human coach is told, on To handle and in the morning
--       brief, when the member's current break is longer than any in their
--       own last 8 weeks while nothing is planned for them. Learning the
--       member's pattern to flag a deviation is profiling (Law 25 art. 8.1),
--       so it is off until the member grants it from Dravr's own app.
--       coach_routine_detection_at records when they last answered: a grant,
--       a revoke, or "not now" on the one-time card that asks for it (a
--       revoke while already off), NULL while they never did, which is when
--       the card is due. Additive; no backfill.
ALTER TABLE coaching_group_members ADD COLUMN coach_routine_detection INTEGER NOT NULL DEFAULT 0; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE coaching_group_members ADD COLUMN coach_routine_detection_at TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
