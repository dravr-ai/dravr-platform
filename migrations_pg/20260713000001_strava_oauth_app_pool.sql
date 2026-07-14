-- ABOUTME: Strava shared-app OAuth credential pool + per-token/per-state app attribution (PostgreSQL)
-- ABOUTME: Mirrors the SQLite migration; column names identical, timestamps BIGINT Unix epoch seconds

-- See the matching SQLite migration for the full rationale. Column NAMES are
-- identical across backends (schema_parity_test asserts this); only the integer
-- width differs (SQLite INTEGER -> PG BIGINT) and enabled is a native BOOLEAN.
CREATE TABLE IF NOT EXISTS strava_oauth_app_pool (
    client_id               TEXT PRIMARY KEY,
    client_secret_encrypted TEXT NOT NULL,
    seat_cap                INTEGER NOT NULL CHECK (seat_cap > 0),
    enabled                 BOOLEAN NOT NULL DEFAULT TRUE,
    label                   TEXT,
    created_at              BIGINT NOT NULL,
    updated_at              BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_strava_oauth_app_pool_enabled ON strava_oauth_app_pool(enabled);

-- Which OAuth app issued each token (NULL = env default / pre-pool legacy).
ALTER TABLE user_oauth_tokens ADD COLUMN IF NOT EXISTS oauth_app_client_id TEXT;

-- The pool app chosen at authorize time, pinned through to the code exchange.
ALTER TABLE oauth_client_states ADD COLUMN IF NOT EXISTS oauth_app_client_id TEXT;
