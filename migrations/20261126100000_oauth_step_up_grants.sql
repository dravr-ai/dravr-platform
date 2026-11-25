-- ABOUTME: Step-up child grants, the Host Profile basis, and the consume and step_up receipt kinds (SQLite)
-- ABOUTME: ONE-WAY: rebuilds oauth_grant_events to widen its event CHECK; the rest is nullable columns and recreated indexes

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Owl phase 5, slice 4b (carnet#1544, plan revisions 15-17).
--
-- oauth_client_grants: a step-up approval records a CHILD of the client's
-- live grant, never an edit of it (owl PDPP §7.6). NULL in every new column
-- for an ordinary (parent) grant.
--   parent_grant_id      the live grant this child was approved under
--   step_up_auth_time    when the athlete signed in on Dravr's hosted page
--                        (unix seconds), at most 300 s before approving
--   step_up_acr          owl's `owner_verified` (a recorded deviation: a
--                        fresh password or Google sign-in, not a passkey)
--   host_profile_basis   how the host satisfied Host Profile T2 (owl §6.2):
--                        declared_cimd | declared_request | assessed
--   host_profile_tier    the tier the basis allows (T2)
ALTER TABLE oauth_client_grants ADD COLUMN parent_grant_id TEXT;
ALTER TABLE oauth_client_grants ADD COLUMN step_up_auth_time INTEGER;
ALTER TABLE oauth_client_grants ADD COLUMN step_up_acr TEXT;
ALTER TABLE oauth_client_grants ADD COLUMN host_profile_basis TEXT;
ALTER TABLE oauth_client_grants ADD COLUMN host_profile_tier TEXT;

-- The one-live-grant-per-scope rule holds for parents only; a child is held
-- to one live child per parent, so a new step-up revokes the old child first.
DROP INDEX IF EXISTS idx_oauth_client_grants_active;
CREATE UNIQUE INDEX IF NOT EXISTS idx_oauth_client_grants_active
    ON oauth_client_grants(user_id, tenant_id, client_id, scope)
    WHERE revoked_at IS NULL AND parent_grant_id IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_oauth_client_grants_live_child
    ON oauth_client_grants(parent_grant_id)
    WHERE revoked_at IS NULL AND parent_grant_id IS NOT NULL;

-- The Host Profile a client's CIMD document declares (`mcp_host_profile`,
-- owl §6.2 carrier A), as JSON {tier_max, version}; NULL when it declares
-- none, and for every client registered by DCR.
ALTER TABLE oauth2_clients ADD COLUMN host_profile TEXT;

-- oauth_grant_events: `consume` (a single-use grant's first read) and
-- `step_up` (a child approved) join issue | revoke | expire (owl §8.1).
-- SQLite cannot alter a CHECK, so the table is rebuilt, every row copied,
-- and all five indexes recreated; `consume` joins the terminal predicate, so
-- a consumed grant has exactly one terminal receipt.
CREATE TABLE oauth_grant_events_rebuilt (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    client_grant_id TEXT NOT NULL,
    event TEXT NOT NULL CHECK (event IN ('issue', 'revoke', 'expire', 'consume', 'step_up')),
    actor TEXT NOT NULL CHECK (actor IN ('user', 'client', 'auto')),
    at TEXT NOT NULL,
    payload TEXT NOT NULL,
    payload_sha256 TEXT NOT NULL,
    jws TEXT,
    logged_at TEXT,
    attempts INTEGER NOT NULL DEFAULT 0,
    poisoned_at TEXT
);
INSERT INTO oauth_grant_events_rebuilt
    (id, tenant_id, user_id, client_grant_id, event, actor, at, payload,
     payload_sha256, jws, logged_at, attempts, poisoned_at)
SELECT id, tenant_id, user_id, client_grant_id, event, actor, at, payload,
       payload_sha256, jws, logged_at, attempts, poisoned_at
FROM oauth_grant_events;
DROP TABLE oauth_grant_events;
ALTER TABLE oauth_grant_events_rebuilt RENAME TO oauth_grant_events;

CREATE INDEX IF NOT EXISTS idx_oauth_grant_events_owner
    ON oauth_grant_events (tenant_id, user_id, at);
CREATE INDEX IF NOT EXISTS idx_oauth_grant_events_grant
    ON oauth_grant_events (client_grant_id, event);
CREATE UNIQUE INDEX IF NOT EXISTS idx_oauth_grant_events_terminal
    ON oauth_grant_events (client_grant_id)
    WHERE event IN ('revoke', 'expire', 'consume');
CREATE INDEX IF NOT EXISTS idx_oauth_grant_events_unlogged
    ON oauth_grant_events (attempts, at) WHERE logged_at IS NULL AND poisoned_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_oauth_grant_events_unsigned
    ON oauth_grant_events (attempts, at) WHERE jws IS NULL AND poisoned_at IS NULL;
