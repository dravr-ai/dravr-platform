-- ABOUTME: family_id on oauth2_refresh_tokens — the rotation chain a token belongs to, so a replay revokes it (SQLite)
-- ABOUTME: Same reuse-detection rule the first-party session_refresh_tokens table has kept since it was created

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A refresh exchange revokes the token it consumes and issues a successor.
-- Presenting a token that was already rotated out is the replay shape of a
-- stolen credential; the OAuth2 server used to answer it with invalid_grant
-- and leave the successor live, while first-party sessions revoked the whole
-- chain. `family_id` ties each chain together so both revoke it (carnet#560).
ALTER TABLE oauth2_refresh_tokens ADD COLUMN family_id TEXT NOT NULL DEFAULT ''; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- Tokens issued before this column existed start a chain of their own: the
-- stored HMAC is unique per row, so each is its own family.
UPDATE oauth2_refresh_tokens SET family_id = token WHERE family_id = '';

CREATE INDEX IF NOT EXISTS idx_oauth2_refresh_tokens_family_id
    ON oauth2_refresh_tokens(family_id);
