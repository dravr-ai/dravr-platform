-- ABOUTME: session_refresh_tokens — the long-lived credential a first-party device holds between 24-hour JWTs (SQLite)
-- ABOUTME: Separate from oauth2_refresh_tokens on purpose; that table is keyed to a registered OAuth client

-- A phone that sits unopened for longer than JWT_EXPIRY_HOURS used to land on
-- the login screen, because the password login issued nothing that outlived
-- the access token. This table holds what it issues now: a refresh token the
-- mobile app trades for a fresh JWT through `grant_type=refresh_token`.
--
-- Not `oauth2_refresh_tokens`: that table's `client_id` is a foreign key to a
-- registered OAuth client and cascades with it, which a first-party login has
-- no counterpart for. Same mechanism, separate token space.
--
-- Only the HMAC of the token is stored, so a database read cannot reconstruct
-- a usable credential. Every use rotates the token and revokes the one it
-- replaced; `family_id` ties the chain together so that presenting a token
-- that was already rotated out — the replay shape of a stolen credential —
-- revokes the whole chain, live member included.

CREATE TABLE IF NOT EXISTS session_refresh_tokens (
    token_hash TEXT PRIMARY KEY,
    family_id  TEXT NOT NULL,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id  TEXT,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    revoked_at TEXT
);

-- Reuse detection and logout revoke by family; a password change revokes by user.
CREATE INDEX IF NOT EXISTS idx_session_refresh_tokens_family_id
    ON session_refresh_tokens(family_id);
CREATE INDEX IF NOT EXISTS idx_session_refresh_tokens_user_id
    ON session_refresh_tokens(user_id);
