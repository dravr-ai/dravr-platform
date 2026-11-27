-- ABOUTME: The athlete's per-coach switch for the missed-session check, and the brief's counts of who it did not check (PostgreSQL)
-- ABOUTME: On by default under coach sharing; a run counts athletes whose calendar was unreadable or who switched it off

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Routines P3a, missed planned sessions (carnet#1792). See the SQLite twin
-- for what each column means.
ALTER TABLE coaching_group_members ADD COLUMN IF NOT EXISTS coach_missed_session_check BOOLEAN NOT NULL DEFAULT TRUE;
ALTER TABLE coaching_group_members ADD COLUMN IF NOT EXISTS coach_missed_session_check_at TIMESTAMPTZ;
ALTER TABLE routine_runs ADD COLUMN IF NOT EXISTS missed_unreadable BIGINT NOT NULL DEFAULT 0;
ALTER TABLE routine_runs ADD COLUMN IF NOT EXISTS missed_off BIGINT NOT NULL DEFAULT 0;
