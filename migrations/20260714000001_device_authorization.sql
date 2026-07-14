-- ABOUTME: Device Authorization Grant (RFC 8628) pending authorizations for `pierre-cli auth login` (SQLite)
-- ABOUTME: Short-lived device_code/user_code rows; a super-admin approves, then the CLI's token poll mints a fresh admin token and deletes the row

-- One row per in-flight `pierre-cli auth login`. The CLI creates a pending row
-- (device_code is stored only as a SHA-256 hash, like an API credential; the
-- user_code is the short code the operator types into the approval page). A
-- super-admin approves it in the browser, which flips status to 'approved' and
-- records their email. The CLI's next token poll (presenting the raw
-- device_code) mints a fresh super-admin admin token and deletes the row
-- (single-use); nothing secret is ever stored here. Unclaimed rows expire via
-- expires_at. Timestamps are Unix epoch seconds (INTEGER) to match the
-- cross-backend convention and avoid DateTime bind hazards. Column NAMES mirror
-- the PostgreSQL migration exactly (schema_parity_test); only integer width differs.
CREATE TABLE IF NOT EXISTS device_authorization (
    device_code_hash TEXT PRIMARY KEY,
    user_code        TEXT NOT NULL UNIQUE,
    status           TEXT NOT NULL DEFAULT 'pending',
    approved_by      TEXT,
    created_at       INTEGER NOT NULL,
    expires_at       INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_device_authorization_user_code ON device_authorization(user_code);
CREATE INDEX IF NOT EXISTS idx_device_authorization_expires_at ON device_authorization(expires_at);
