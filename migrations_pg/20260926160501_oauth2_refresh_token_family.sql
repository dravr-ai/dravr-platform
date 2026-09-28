-- ABOUTME: family_id on oauth2_refresh_tokens — the rotation chain a token belongs to, so a replay revokes it (PostgreSQL)
-- ABOUTME: Same reuse-detection rule the first-party session_refresh_tokens table has kept since it was created

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the matching SQLite migration for the full rationale. Summary: a replayed
-- OAuth2 refresh token now revokes its whole rotation chain (carnet#560).
ALTER TABLE oauth2_refresh_tokens ADD COLUMN IF NOT EXISTS family_id TEXT;

-- Tokens issued before this column existed start a chain of their own.
UPDATE oauth2_refresh_tokens SET family_id = token WHERE family_id IS NULL;

ALTER TABLE oauth2_refresh_tokens ALTER COLUMN family_id SET NOT NULL;

CREATE INDEX IF NOT EXISTS idx_oauth2_refresh_tokens_family_id
    ON oauth2_refresh_tokens(family_id);
