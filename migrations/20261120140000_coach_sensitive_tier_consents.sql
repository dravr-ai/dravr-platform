-- ABOUTME: Lets a member share their health records (T2) and intimate health records (T3) with their group's human coach (SQLite)
-- ABOUTME: Adds a flag, its stamp and an optional lapse instant per tier to coaching_group_members; ADD COLUMN only (carnet#1544, row 64)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Two consents the member gives their group's human coach beside
-- coach_wellness_read, each off until the member grants it from Dravr's own
-- app and revocable at any time:
--
--   coach_sensitive_t2_read  the coach may read the member's health records
--                            (owl tier T2: labs, clinical records,
--                            self-measured glucose).
--   coach_sensitive_t3_read  the coach may read the member's intimate health
--                            records (owl tier T3: cycle tracking, mental and
--                            reproductive health).
--
-- Each *_at records when the member last granted or revoked it, NULL while
-- it was never granted. Each *_until is when a grant lapses (owl's preset
-- window, 30 days), NULL while it holds until revoked. The member's own
-- consent to AI use of the tier still applies on top. Additive; no backfill:
-- every existing membership starts with neither shared.
--
-- Only the T2 columns are read and written today. The T3 columns land now,
-- unread, so the T3 switch-on (Owl phase 5 slice 1b) needs no second
-- migration on this table.
ALTER TABLE coaching_group_members ADD COLUMN coach_sensitive_t2_read INTEGER NOT NULL DEFAULT 0; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE coaching_group_members ADD COLUMN coach_sensitive_t2_read_at TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE coaching_group_members ADD COLUMN coach_sensitive_t2_read_until TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE coaching_group_members ADD COLUMN coach_sensitive_t3_read INTEGER NOT NULL DEFAULT 0; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE coaching_group_members ADD COLUMN coach_sensitive_t3_read_at TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE coaching_group_members ADD COLUMN coach_sensitive_t3_read_until TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
