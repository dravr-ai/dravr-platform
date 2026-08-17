-- ABOUTME: Re-run of the orphaned-provider_connections cleanup for rows orphaned by the /mcp path.
-- ABOUTME: Deletes oauth/manual connection rows that have no backing user_oauth_tokens row.

-- The 20260714000002 reconciliation healed the orphans that existed then, but
-- one asymmetric disconnect path survived it: the /mcp + SSE carve-out
-- (handle_tenant_disconnect_provider) — which also serves the ACP-headless
-- chat loopback — kept deleting only the user_oauth_tokens row, leaving the
-- provider_connections row behind (dravr-carnet#29). That path now routes
-- through OAuthService::disconnect_provider, which deletes both in lockstep;
-- this re-run heals the rows orphaned between the two fixes.
--
-- Scope: only 'oauth' and 'manual' connections require a backing token
-- ('manual' is how sciotte/sciotte_garmin register). 'synthetic' connections are
-- tokenless by design and MUST be spared.
--
-- user_oauth_tokens.user_id is UUID in Postgres while provider_connections.user_id
-- is TEXT, so both sides are cast to TEXT for a backend-portable comparison.
DELETE FROM provider_connections
WHERE connection_type IN ('oauth', 'manual')
  AND NOT EXISTS (
    SELECT 1
    FROM user_oauth_tokens t
    WHERE CAST(t.user_id AS TEXT) = CAST(provider_connections.user_id AS TEXT)
      AND CAST(t.tenant_id AS TEXT) = CAST(provider_connections.tenant_id AS TEXT)
      AND t.provider = provider_connections.provider
  );
