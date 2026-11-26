-- ABOUTME: Step-up child grants, the Host Profile basis, and the consume and step_up receipt kinds (PostgreSQL)
-- ABOUTME: ONE-WAY: widens oauth_grant_events' event CHECK and terminal index; the rest is nullable columns and recreated indexes

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Owl phase 5, slice 4b (carnet#1544, plan revisions 15-17). The SQLite
-- twin documents each column; the two schemas match.
ALTER TABLE oauth_client_grants ADD COLUMN IF NOT EXISTS parent_grant_id TEXT;
ALTER TABLE oauth_client_grants ADD COLUMN IF NOT EXISTS step_up_auth_time BIGINT;
ALTER TABLE oauth_client_grants ADD COLUMN IF NOT EXISTS step_up_acr TEXT;
ALTER TABLE oauth_client_grants ADD COLUMN IF NOT EXISTS host_profile_basis TEXT;
ALTER TABLE oauth_client_grants ADD COLUMN IF NOT EXISTS host_profile_tier TEXT;

DROP INDEX IF EXISTS idx_oauth_client_grants_active;
CREATE UNIQUE INDEX IF NOT EXISTS idx_oauth_client_grants_active
    ON oauth_client_grants(user_id, tenant_id, client_id, scope)
    WHERE revoked_at IS NULL AND parent_grant_id IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_oauth_client_grants_live_child
    ON oauth_client_grants(parent_grant_id)
    WHERE revoked_at IS NULL AND parent_grant_id IS NOT NULL;

ALTER TABLE oauth2_clients ADD COLUMN IF NOT EXISTS host_profile TEXT;

ALTER TABLE oauth_grant_events DROP CONSTRAINT IF EXISTS oauth_grant_events_event_check;
ALTER TABLE oauth_grant_events ADD CONSTRAINT oauth_grant_events_event_check
    CHECK (event IN ('issue', 'revoke', 'expire', 'consume', 'step_up'));

DROP INDEX IF EXISTS idx_oauth_grant_events_terminal;
CREATE UNIQUE INDEX IF NOT EXISTS idx_oauth_grant_events_terminal
    ON oauth_grant_events (client_grant_id)
    WHERE event IN ('revoke', 'expire', 'consume');
