-- ABOUTME: email_verification_tokens + users.email_verified_at — proves an address belongs to whoever typed it (PostgreSQL)
-- ABOUTME: Mirrors the SQLite migration with PG-native timestamp types

-- See the matching SQLite migration for the full rationale. Summary: registration
-- never proved an address, the only inbox proof ran after signup for people who
-- had already registered, and this closes that. Mechanism is the password-reset
-- `<selector>.<verifier>` scheme — selector plaintext and indexed, verifier stored
-- only as a SHA-256 hash, `attempt_count` for lockout.
--
-- Deliberately a separate token space from password_reset_tokens: resetting a
-- password and verifying an address are different capabilities, and invalidating
-- one set must not clear the other.
--
-- Column types mirror password_reset_tokens exactly: `id` is TEXT, `user_id` is
-- native UUID because `users.id` is UUID here (SQLite keeps both as TEXT). A TEXT
-- FK against a UUID primary key is rejected at migration time, and binding a
-- stringified uuid into a UUID column is the recurring PG decode trap — so the
-- repository binds `Uuid` directly on this backend and `to_string()` on SQLite.

CREATE TABLE IF NOT EXISTS email_verification_tokens (
    id            TEXT PRIMARY KEY,
    user_id       UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    selector      TEXT NOT NULL,
    token_hash    TEXT NOT NULL,
    expires_at    TIMESTAMPTZ NOT NULL,
    used_at       TIMESTAMPTZ,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Consumption is a selector lookup; the rate limiter counts recent rows per user.
CREATE UNIQUE INDEX IF NOT EXISTS idx_email_verification_tokens_selector
    ON email_verification_tokens(selector);
CREATE INDEX IF NOT EXISTS idx_email_verification_tokens_user_id
    ON email_verification_tokens(user_id);
CREATE INDEX IF NOT EXISTS idx_email_verification_tokens_expires_at
    ON email_verification_tokens(expires_at);

-- NULL = never proven. Set once, on first successful verification.
ALTER TABLE users ADD COLUMN IF NOT EXISTS email_verified_at TIMESTAMPTZ;
