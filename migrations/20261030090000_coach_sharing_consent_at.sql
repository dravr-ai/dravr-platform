-- ABOUTME: Records when a member last granted or revoked sharing with their coach (SQLite)
-- ABOUTME: Adds the nullable coach_sharing_consent_at instant to coaching_group_members (carnet#1261)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- When the member last granted or revoked coach_sharing_consent, so the coach
-- roster can say when an athlete stopped sharing. NULL while unknown: a
-- membership from before this column existed keeps NULL, never a guessed
-- date (carnet#1261). Additive and nullable; no backfill.
ALTER TABLE coaching_group_members ADD COLUMN coach_sharing_consent_at TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
