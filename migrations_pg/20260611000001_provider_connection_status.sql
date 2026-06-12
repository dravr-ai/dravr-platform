-- ABOUTME: Add lifecycle status to provider_connections so a dead OAuth refresh marks a
-- ABOUTME: connection needs_reauth (single source of truth) instead of silently looking connected.

-- status: 'active' | 'needs_reauth' | 'revoked'. Presence in this table still means
-- "connected at some point"; status distinguishes a usable connection from one whose
-- token refresh failed non-recoverably and now requires the user to re-authorize.
ALTER TABLE provider_connections ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'active';

-- When status last transitioned. Drives notify dedup (one nudge per active->needs_reauth)
-- and lets the UI show "disconnected since ...".
ALTER TABLE provider_connections ADD COLUMN IF NOT EXISTS status_changed_at TIMESTAMPTZ;

-- Short classification of the last refresh failure (e.g. 'invalid_request', 'invalid_grant').
-- NEVER stores token material — only the OAuth error code / reason class.
ALTER TABLE provider_connections ADD COLUMN IF NOT EXISTS last_error TEXT;

-- When the user was last notified about the needs_reauth transition. NULL until notified;
-- cleared on reconnect so a future disconnect notifies again.
ALTER TABLE provider_connections ADD COLUMN IF NOT EXISTS notified_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_provider_connections_status
    ON provider_connections(status);
