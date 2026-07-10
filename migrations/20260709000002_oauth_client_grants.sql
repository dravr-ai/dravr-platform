-- ABOUTME: oauth_client_grants table — remembers a user's consent to an MCP OAuth client (SQLite)
-- ABOUTME: Backs the consent screen so a re-authorization for an already-approved (client, scope) skips the prompt

-- When a user approves an MCP client on the OAuth consent screen, one row is
-- written here. On a later `/oauth2/authorize` for the same (user, tenant,
-- client_id, scope) the server finds an un-revoked grant and mints the code
-- without re-prompting. Revoking a connected app sets `revoked_at` (soft delete,
-- preserving the audit trail) — a revoked grant no longer matches, so the next
-- authorization shows the consent screen again.
--
-- `scope` is the exact space-separated scope string that was consented to; a
-- request for a *different* scope set does not match and re-prompts (the user is
-- consenting to a specific capability set, not the client in the abstract).
-- id / user_id / tenant_id / client_id are TEXT to match the codebase's
-- native-UUID-bind-trap avoidance (VARCHAR tenant ids decoded as native UUID
-- silently break binds).

CREATE TABLE IF NOT EXISTS oauth_client_grants (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL,
    tenant_id   TEXT NOT NULL,
    client_id   TEXT NOT NULL,
    scope       TEXT NOT NULL,
    granted_at  TEXT NOT NULL DEFAULT (datetime('now')),
    revoked_at  TEXT
);

-- One active grant per (user, tenant, client, scope). Partial unique so a revoked
-- grant does not block re-consenting later.
CREATE UNIQUE INDEX IF NOT EXISTS idx_oauth_client_grants_active
    ON oauth_client_grants(user_id, tenant_id, client_id, scope)
    WHERE revoked_at IS NULL;

-- Grant lookup on authorize and the user's "connected apps" list both filter by owner.
CREATE INDEX IF NOT EXISTS idx_oauth_client_grants_user
    ON oauth_client_grants(user_id, tenant_id);
