-- ABOUTME: Gives an MCP OAuth grant its resolved authorization details and expiry, and names it on what it mints (SQLite)
-- ABOUTME: Adds the grant's id to codes, refresh tokens and JWT usage, and the RFC 9126 pushed-request store (carnet#1544)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- oauth_client_grants.authorization_details
--                  the RFC 9396 details the authorization server resolved the
--                  grant to, as one JSON document. NULL is a plain grant: every
--                  grant recorded before this column, and every one given
--                  without details, reaches what a grant reached before.
-- oauth_client_grants.expires_at
--                  when the grant stops admitting a token; NULL never expires,
--                  as every grant before this column.
-- client_grant_id  the oauth_client_grants row a code, a refresh token or a
--                  request was issued under. NULL on everything minted before
--                  this column, which mints and admits as before.
-- Additive and nullable; no backfill.
ALTER TABLE oauth_client_grants ADD COLUMN authorization_details TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE oauth_client_grants ADD COLUMN expires_at TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE oauth2_auth_codes ADD COLUMN client_grant_id TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE oauth2_refresh_tokens ADD COLUMN client_grant_id TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE jwt_usage ADD COLUMN client_grant_id TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- A grant's requests in a window, for its hourly budget; only rows a grant
-- was named on are indexed.
CREATE INDEX IF NOT EXISTS idx_jwt_usage_client_grant
    ON jwt_usage (user_id, client_grant_id, timestamp)
    WHERE client_grant_id IS NOT NULL;

-- RFC 9126 pushed authorization requests. An authorization server protocol
-- table, like oauth2_clients: written before any user is known, so it has
-- neither tenant_id nor user_id, and every query names request_uri_hash and
-- client_id. No foreign key to oauth2_clients: a client known by its
-- metadata document has no row until an athlete signs in.
-- request_uri_hash  SHA-256 of the request_uri handed to the client; the URI
--                   itself is never stored.
-- parameters        the checked authorization request, as JSON.
-- used_at           when a code was minted from it (single use, RFC 9126 §4).
CREATE TABLE IF NOT EXISTS oauth2_pushed_requests (
    id TEXT PRIMARY KEY,
    request_uri_hash TEXT NOT NULL UNIQUE,
    client_id TEXT NOT NULL,
    parameters TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    used_at TEXT,
    created_at TEXT NOT NULL
);
