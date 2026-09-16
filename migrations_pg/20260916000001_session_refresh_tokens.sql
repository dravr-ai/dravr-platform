-- ABOUTME: session_refresh_tokens — the long-lived credential a first-party device holds between 24-hour JWTs (PostgreSQL)
-- ABOUTME: Mirrors the SQLite migration with PG-native UUID and timestamp types

-- See the matching SQLite migration for the full rationale. Summary: the
-- password login issued nothing that outlived the 24-hour JWT, so a phone left
-- closed for a day landed on the login screen. This holds the refresh token the
-- mobile app now trades for a fresh JWT. Only the token's HMAC is stored; every
-- use rotates it, and `family_id` lets a replayed token revoke its whole chain.
--
-- `user_id` is native UUID because `users.id` is UUID here (SQLite keeps both
-- as TEXT), so the repository binds `Uuid` directly on this backend.

CREATE TABLE IF NOT EXISTS session_refresh_tokens (
    token_hash TEXT PRIMARY KEY,
    family_id  TEXT NOT NULL,
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id  TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ
);

-- Reuse detection and logout revoke by family; a password change revokes by user.
CREATE INDEX IF NOT EXISTS idx_session_refresh_tokens_family_id
    ON session_refresh_tokens(family_id);
CREATE INDEX IF NOT EXISTS idx_session_refresh_tokens_user_id
    ON session_refresh_tokens(user_id);
