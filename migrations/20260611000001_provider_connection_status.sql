-- ABOUTME: Add lifecycle status to provider_connections so a dead OAuth refresh marks a
-- ABOUTME: connection needs_reauth (single source of truth) instead of silently looking connected.

-- SQLite has no `ADD COLUMN IF NOT EXISTS` (Postgres-only; see the migrations_pg twin).
-- Each ADD COLUMN below carries `-- idempotency-ok` because the column is brand new in
-- this migration and the dev SQLite DB is rebuilt from scratch (reset-dev-db.sh), so
-- drift-on-add cannot occur here the way it can on a long-lived Postgres instance.

-- status: 'active' | 'needs_reauth' | 'revoked'. Presence in this table still means
-- "connected at some point"; status distinguishes a usable connection from one whose
-- token refresh failed non-recoverably and now requires the user to re-authorize.
ALTER TABLE provider_connections ADD COLUMN status TEXT NOT NULL DEFAULT 'active';  -- idempotency-ok: SQLite lacks ADD COLUMN IF NOT EXISTS; column is new here

-- When status last transitioned. Drives notify dedup (one nudge per active->needs_reauth)
-- and lets the UI show "disconnected since ...".
ALTER TABLE provider_connections ADD COLUMN status_changed_at TEXT;  -- idempotency-ok: SQLite lacks ADD COLUMN IF NOT EXISTS; column is new here

-- Short classification of the last refresh failure (e.g. 'invalid_request', 'invalid_grant').
-- NEVER stores token material — only the OAuth error code / reason class.
ALTER TABLE provider_connections ADD COLUMN last_error TEXT;  -- idempotency-ok: SQLite lacks ADD COLUMN IF NOT EXISTS; column is new here

-- When the user was last notified about the needs_reauth transition. NULL until notified;
-- cleared on reconnect so a future disconnect notifies again.
ALTER TABLE provider_connections ADD COLUMN notified_at TEXT;  -- idempotency-ok: SQLite lacks ADD COLUMN IF NOT EXISTS; column is new here

CREATE INDEX IF NOT EXISTS idx_provider_connections_status
    ON provider_connections(status);
