-- ABOUTME: family_created_at on oauth2_refresh_tokens — when a rotation chain began, for an absolute family lifetime (SQLite)
-- ABOUTME: Additive and nullable: NULL is a legacy family with no absolute limit, so the previous image keeps working

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A refresh token slides: every rotation grants a fresh window, so a forgotten
-- connection could refresh forever. The first token of a chain stamps the
-- chain's start, every successor carries it, and the exchange refuses a chain
-- older than the absolute family lifetime so the athlete consents again.
-- Tokens issued before this column keep NULL: no absolute limit, exactly the
-- behaviour they were issued under. No backfill.
ALTER TABLE oauth2_refresh_tokens ADD COLUMN family_created_at TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
