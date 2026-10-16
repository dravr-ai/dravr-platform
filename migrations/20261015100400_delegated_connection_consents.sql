-- ABOUTME: Per-link consents on delegated_connections — what the member lets a confirmed link do beyond reading workouts (SQLite)
-- ABOUTME: Additive only: calendar_write and wellness_read, each off until the member grants it, with when each last changed

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Confirming a link is the member's consent to having their workouts on the
-- coaching platform read through the coach's account, and to nothing more.
-- Two further consents are the member's alone, per link: each is granted only
-- by an explicit choice, at confirm or on the confirmed link, from Dravr's own
-- app, and the member can revoke it at any time.
--
--   calendar_write  the link's coach writes the member's training calendar on
--                   the platform; offered where the platform takes a coach's
--                   calendar writes.
--   wellness_read   the member's sleep, HRV, resting heart rate, weight and
--                   daily note sync through the coach's account; offered where
--                   the coach's credential reads them. Revoking it deletes the
--                   wellness rows the link wrote.
--
-- Every existing link starts with both off: none was confirmed on wording that
-- named either. Each *_at records when the member last granted or revoked it,
-- NULL while it was never granted.
ALTER TABLE delegated_connections ADD COLUMN calendar_write INTEGER NOT NULL DEFAULT 0; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE delegated_connections ADD COLUMN calendar_write_at TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE delegated_connections ADD COLUMN wellness_read INTEGER NOT NULL DEFAULT 0; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE delegated_connections ADD COLUMN wellness_read_at TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
