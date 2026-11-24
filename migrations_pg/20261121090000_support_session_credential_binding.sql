-- ABOUTME: Binds an admin's support session to the one credential that started it over MCP or A2A (PostgreSQL)
-- ABOUTME: Adds impersonation_sessions.grant_id and api_key_id: NULL for a web session, else the grant or key (carnet#1511)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A support session an admin's assistant starts acts as the user only for
-- the credential that started it (JF, Q21): at most one of these is set.
--
-- grant_id: the OAuth grant, as the refresh-token family its access token
-- was minted in (oauth2_refresh_tokens.family_id). A family is one
-- authorization: a second connection of the same application is another
-- family, so it acts as the admin. Revoking the family, the application's
-- consent, or the application ends the session.
--
-- api_key_id: one of the admin's API keys (api_keys.id). Another key of the
-- same admin acts as the admin; deactivating the key ends the session.
--
-- Both NULL for a session the admin console started on the web, which never
-- turns an assistant connection into the user. Additive; no backfill: every
-- existing row is a web session. No foreign keys: an ended session keeps
-- naming the credential on the audit trail after the credential is gone.
ALTER TABLE impersonation_sessions ADD COLUMN IF NOT EXISTS grant_id TEXT;
ALTER TABLE impersonation_sessions ADD COLUMN IF NOT EXISTS api_key_id TEXT;

-- The revocation paths end the live sessions of a credential by these.
CREATE INDEX IF NOT EXISTS idx_impersonation_sessions_grant
    ON impersonation_sessions(grant_id) WHERE grant_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_impersonation_sessions_api_key
    ON impersonation_sessions(api_key_id) WHERE api_key_id IS NOT NULL;
