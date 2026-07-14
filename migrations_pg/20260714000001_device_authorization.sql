-- ABOUTME: Device Authorization Grant (RFC 8628) pending authorizations for `pierre-cli auth login` (PostgreSQL)
-- ABOUTME: Mirrors the SQLite migration; column names identical, timestamps BIGINT Unix epoch seconds

-- See the matching SQLite migration for the full rationale. Column NAMES are
-- identical across backends (schema_parity_test asserts this); only the integer
-- width differs (SQLite INTEGER -> PG BIGINT).
CREATE TABLE IF NOT EXISTS device_authorization (
    device_code_hash TEXT PRIMARY KEY,
    user_code        TEXT NOT NULL UNIQUE,
    status           TEXT NOT NULL DEFAULT 'pending',
    approved_by      TEXT,
    created_at       BIGINT NOT NULL,
    expires_at       BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_device_authorization_user_code ON device_authorization(user_code);
CREATE INDEX IF NOT EXISTS idx_device_authorization_expires_at ON device_authorization(expires_at);
