-- ABOUTME: last_authorized_at on oauth2_clients — when a refresh token was last issued through the client (SQLite)
-- ABOUTME: NULL marks a registration no user finished authorizing; the retention sweep and the pending ceiling read it
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- `POST /oauth2/register` is anonymous (RFC 7591), and every row it wrote was
-- kept for good: `expires_at` stopped an expired client from being used but
-- nothing ever deleted one (carnet#483). A client a user connected and a
-- registration nobody finished looked the same, so neither could be reclaimed
-- before its year-long expiry. This column tells them apart. It is written in
-- the same transaction as each refresh token, and a refresh token is only ever
-- issued for a user — so NULL means no user has authorized the client.
ALTER TABLE oauth2_clients ADD COLUMN last_authorized_at TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- Clients registered before this column existed: a client that issued a refresh
-- token was authorized, most recently when it issued the newest one. Rotation
-- revokes a refresh token rather than deleting it, so the newest row survives
-- until its user is deleted. A client with none left stays NULL: nobody it could
-- still serve ever authorized it.
UPDATE oauth2_clients
SET last_authorized_at = (
    SELECT MAX(r.created_at)
    FROM oauth2_refresh_tokens r
    WHERE r.client_id = oauth2_clients.client_id
)
WHERE last_authorized_at IS NULL;

-- The pending set: counted on every registration, scanned by age on every
-- sweep. Partial, so it holds only the rows neither ever looks past.
CREATE INDEX IF NOT EXISTS idx_oauth2_clients_pending_created_at
    ON oauth2_clients(created_at)
    WHERE last_authorized_at IS NULL;

-- The sweep's other cutoff: registrations past their expiry grace.
CREATE INDEX IF NOT EXISTS idx_oauth2_clients_expires_at ON oauth2_clients(expires_at);
