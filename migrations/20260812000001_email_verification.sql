-- ABOUTME: email_verification_tokens + users.email_verified_at — proves an address belongs to whoever typed it (SQLite)
-- ABOUTME: Separate token space from password_reset_tokens on purpose; a reset token must never verify an address

-- Registration accepted any address and never proved it. The only inbox proof
-- in the platform was the messaging OTP flow, which runs *after* signup and only
-- for people who already registered — so the strongest identity check sat behind
-- the weakest. This table closes that.
--
-- Mechanism is the password-reset one: a `<selector>.<verifier>` token where only
-- the selector (plaintext, indexed) and the verifier's SHA-256 hash are stored, so
-- a database read cannot reconstruct a usable token. `attempt_count` gives the same
-- lockout the reset flow has (CWE-307).
--
-- Deliberately NOT the password_reset_tokens table with a `purpose` column: a
-- token that can reset a password and a token that can verify an address are
-- different capabilities, and `invalidate_tokens(user_id)` on one must not clear
-- the other. Same mechanism, separate token spaces.
--
-- `email_verified_at` lands on `users` rather than being inferred from a consumed
-- token, because the token is single-use and swept — the fact that an address was
-- proven has to outlive its proof.

CREATE TABLE IF NOT EXISTS email_verification_tokens (
    id            TEXT PRIMARY KEY,
    user_id       TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    selector      TEXT NOT NULL,
    token_hash    TEXT NOT NULL,
    expires_at    TEXT NOT NULL,
    used_at       TEXT,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Consumption is a selector lookup; the rate limiter counts recent rows per user.
CREATE UNIQUE INDEX IF NOT EXISTS idx_email_verification_tokens_selector
    ON email_verification_tokens(selector);
CREATE INDEX IF NOT EXISTS idx_email_verification_tokens_user_id
    ON email_verification_tokens(user_id);
CREATE INDEX IF NOT EXISTS idx_email_verification_tokens_expires_at
    ON email_verification_tokens(expires_at);

-- NULL = never proven. Set once, on first successful verification.
ALTER TABLE users ADD COLUMN email_verified_at TEXT; -- idempotency-ok: SQLite has no ADD COLUMN IF NOT EXISTS; brand-new column, PG mirror uses IF NOT EXISTS
