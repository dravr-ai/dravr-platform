-- ABOUTME: Lets a member share their health records (T2) and intimate health records (T3) with their group's human coach (PostgreSQL)
-- ABOUTME: Adds a flag, its stamp and an optional lapse instant per tier to coaching_group_members; ADD COLUMN only (carnet#1544, row 64)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the matching SQLite migration for the full rationale. Each flag is off
-- until the member grants it; *_at is when they last granted or revoked it;
-- *_until is when a grant lapses, NULL while it holds until revoked.
-- Additive; no backfill. Only the T2 columns are read and written today; the
-- T3 columns land now, unread, so the T3 switch-on (Owl phase 5 slice 1b)
-- needs no second migration on this table.
ALTER TABLE coaching_group_members ADD COLUMN IF NOT EXISTS coach_sensitive_t2_read BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE coaching_group_members ADD COLUMN IF NOT EXISTS coach_sensitive_t2_read_at TIMESTAMPTZ;
ALTER TABLE coaching_group_members ADD COLUMN IF NOT EXISTS coach_sensitive_t2_read_until TIMESTAMPTZ;
ALTER TABLE coaching_group_members ADD COLUMN IF NOT EXISTS coach_sensitive_t3_read BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE coaching_group_members ADD COLUMN IF NOT EXISTS coach_sensitive_t3_read_at TIMESTAMPTZ;
ALTER TABLE coaching_group_members ADD COLUMN IF NOT EXISTS coach_sensitive_t3_read_until TIMESTAMPTZ;
