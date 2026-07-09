-- ABOUTME: Add selector + attempt_count to password_reset_tokens (selector/verifier scheme)
-- ABOUTME: Enables per-token lookup (no global hash match) + per-code brute-force lockout (T3MP3ST F1)

-- `selector` is the plaintext lookup half of a `<selector>.<verifier>` token; `token_hash`
-- now stores SHA-256 of the VERIFIER half. Lookups key on `selector` (unique) instead of a
-- global `token_hash` scan, and `attempt_count` bounds wrong-verifier guesses per token.
ALTER TABLE password_reset_tokens ADD COLUMN IF NOT EXISTS selector TEXT;
ALTER TABLE password_reset_tokens ADD COLUMN IF NOT EXISTS attempt_count INTEGER NOT NULL DEFAULT 0;

-- Unique so a selector maps to exactly one live token. Postgres treats NULLs as distinct,
-- so pre-existing rows (selector IS NULL) do not collide.
CREATE UNIQUE INDEX IF NOT EXISTS idx_password_reset_tokens_selector
    ON password_reset_tokens(selector);
