-- ABOUTME: The athlete's per-coach switch for the missed-session check, and the brief's counts of who it did not check
-- ABOUTME: On by default under coach sharing; a run counts athletes whose calendar was unreadable or who switched it off

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Routines P3a, missed planned sessions (carnet#1792).
--
--   coaching_group_members.coach_missed_session_check
--       the group's human coach is told, on To handle and in the morning
--       brief, when none of the sessions planned on consecutive planned days
--       has a matching activity. On by default under coach sharing (Phil,
--       2026-11-23: a plain comparison the athlete can dismiss); the member
--       turns it off per membership. coach_missed_session_check_at records
--       when they last turned it on or off, NULL while they never did.
--
--   routine_runs.missed_unreadable / missed_off
--       how many listed-or-not athletes a morning brief did not check for
--       missed sessions: a calendar could not be read, or they switched the
--       check off. Additive; no backfill.
ALTER TABLE coaching_group_members ADD COLUMN coach_missed_session_check INTEGER NOT NULL DEFAULT 1; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE coaching_group_members ADD COLUMN coach_missed_session_check_at TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE routine_runs ADD COLUMN missed_unreadable INTEGER NOT NULL DEFAULT 0; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE routine_runs ADD COLUMN missed_off INTEGER NOT NULL DEFAULT 0; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
