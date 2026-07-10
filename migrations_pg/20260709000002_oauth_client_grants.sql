-- ABOUTME: oauth_client_grants table — remembers a user's consent to an MCP OAuth client (PostgreSQL)
-- ABOUTME: Mirrors the SQLite migration with PG-native TIMESTAMPTZ; backs the consent screen's remember/revoke

-- See the matching SQLite migration for the full rationale. Summary: one row per
-- user-approved MCP OAuth client; `/oauth2/authorize` finds an un-revoked grant
-- for the same (user, tenant, client_id, scope) and skips the consent prompt.
-- Revoking a connected app sets `revoked_at` (soft delete, keeps the audit trail).
--
-- id / user_id / tenant_id / client_id are TEXT (not native UUID) to dodge the
-- VARCHAR-tenant-id-decoded-as-native-UUID bind trap that recurs in this codebase.

CREATE TABLE IF NOT EXISTS oauth_client_grants (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL,
    tenant_id   TEXT NOT NULL,
    client_id   TEXT NOT NULL,
    scope       TEXT NOT NULL,
    granted_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at  TIMESTAMPTZ
);

-- One active grant per (user, tenant, client, scope). Partial unique so a revoked
-- grant does not block re-consenting later.
CREATE UNIQUE INDEX IF NOT EXISTS idx_oauth_client_grants_active
    ON oauth_client_grants(user_id, tenant_id, client_id, scope)
    WHERE revoked_at IS NULL;

-- Grant lookup on authorize and the user's "connected apps" list both filter by owner.
CREATE INDEX IF NOT EXISTS idx_oauth_client_grants_user
    ON oauth_client_grants(user_id, tenant_id);
