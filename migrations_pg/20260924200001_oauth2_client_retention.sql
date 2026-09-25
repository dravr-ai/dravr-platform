-- ABOUTME: last_authorized_at on oauth2_clients — when a refresh token was last issued through the client (PostgreSQL)
-- ABOUTME: NULL marks a registration no user finished authorizing; the retention sweep and the pending ceiling read it
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the matching SQLite migration for the full rationale. Summary: dynamic
-- registrations were never deleted (carnet#483); this column separates a client
-- a user authorized (a refresh token was issued through it) from a registration
-- nobody finished, which retention can reclaim within a day.
ALTER TABLE oauth2_clients ADD COLUMN IF NOT EXISTS last_authorized_at TIMESTAMPTZ;

-- Clients registered before this column existed: stamped with their newest
-- refresh token, which rotation revokes rather than deletes. A client with none
-- left stays NULL.
UPDATE oauth2_clients
SET last_authorized_at = (
    SELECT MAX(r.created_at)
    FROM oauth2_refresh_tokens r
    WHERE r.client_id = oauth2_clients.client_id
)
WHERE last_authorized_at IS NULL;

-- The pending set, counted on every registration and scanned by age on every sweep.
CREATE INDEX IF NOT EXISTS idx_oauth2_clients_pending_created_at
    ON oauth2_clients(created_at)
    WHERE last_authorized_at IS NULL;

-- The sweep's other cutoff: registrations past their expiry grace.
CREATE INDEX IF NOT EXISTS idx_oauth2_clients_expires_at ON oauth2_clients(expires_at);
