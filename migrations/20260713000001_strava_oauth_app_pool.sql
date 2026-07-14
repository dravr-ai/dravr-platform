-- ABOUTME: Strava shared-app OAuth credential pool + per-token/per-state app attribution (SQLite)
-- ABOUTME: Operators add extra Strava API apps to grow the athlete-seat cap; each token records its issuing app so refresh uses that app's secret

-- Pool of platform-owned Strava OAuth apps, IN ADDITION to the env
-- STRAVA_CLIENT_ID app (which stays the implicit default member). Strava caps
-- the number of athletes a single app may connect and does not expose it via
-- the API, so more approved apps = more OAuth seats before falling back to the
-- Sciotte mirror. client_secret is encrypted at rest (AES-256-GCM, the same
-- envelope used for user_oauth_tokens); client_id is public (it appears in the
-- authorize URL) and is the primary key. Timestamps are Unix epoch seconds
-- (INTEGER) to match the current cross-backend convention and avoid DateTime
-- bind hazards. Column NAMES mirror the PostgreSQL migration exactly
-- (schema_parity_test); only the integer width differs.
CREATE TABLE IF NOT EXISTS strava_oauth_app_pool (
    client_id               TEXT PRIMARY KEY,
    client_secret_encrypted TEXT NOT NULL,
    seat_cap                INTEGER NOT NULL CHECK (seat_cap > 0),
    enabled                 INTEGER NOT NULL DEFAULT 1,
    label                   TEXT,
    created_at              INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_strava_oauth_app_pool_enabled ON strava_oauth_app_pool(enabled);

-- Which OAuth app issued each token. NULL = the env default app or a pre-pool
-- legacy token (both refresh with the env STRAVA_CLIENT_SECRET). A non-NULL
-- value names a strava_oauth_app_pool.client_id whose secret refresh must use.
ALTER TABLE user_oauth_tokens ADD COLUMN oauth_app_client_id TEXT; -- idempotency-ok: SQLite has no ADD COLUMN IF NOT EXISTS; brand-new column, PG mirror uses IF NOT EXISTS

-- The pool app chosen at authorize time, pinned through to the code exchange so
-- the exchange uses the same client_id/secret that built the authorize URL
-- (a mismatch makes Strava reject the code). NULL = env default app.
ALTER TABLE oauth_client_states ADD COLUMN oauth_app_client_id TEXT; -- idempotency-ok: SQLite has no ADD COLUMN IF NOT EXISTS; brand-new column, PG mirror uses IF NOT EXISTS
