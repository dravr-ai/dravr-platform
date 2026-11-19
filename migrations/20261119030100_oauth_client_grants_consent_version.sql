-- ABOUTME: Records which consent text a user approved an MCP OAuth client under, and in which language (SQLite)
-- ABOUTME: Adds nullable consent_version and consent_locale to oauth_client_grants, so a changed text asks again (carnet#1660)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- consent_version  a digest of every line the consent screen can show, in
--                  consent_locale, at the moment the user chose Allow access
--                  (pierre-routes-identity oauth2/consent_version.rs). An
--                  authorization skips the screen only while the text in
--                  that language is still the one approved.
-- consent_locale   the language the screen was shown in.
-- NULL for every grant recorded before these columns: the text those users
-- read is not known, so their next authorization shows the screen again
-- (EFVP 2026-11-17, decision 9.4a). Additive and nullable; no backfill.
ALTER TABLE oauth_client_grants ADD COLUMN consent_version TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE oauth_client_grants ADD COLUMN consent_locale TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
