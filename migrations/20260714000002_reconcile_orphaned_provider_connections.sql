-- ABOUTME: One-time cleanup of orphaned provider_connections left by asymmetric disconnects.
-- ABOUTME: Deletes oauth/manual connection rows that have no backing user_oauth_tokens row.

-- Some disconnect paths (the sciotte session disconnect and the chat
-- disconnect_provider tool) historically deleted the user_oauth_tokens row but
-- left the provider_connections row behind. The orphan makes the UI report
-- "Connected" for a session that no longer exists, and routes later fetches to a
-- backend with no token — for Garmin that falls through to the uncredentialed
-- OAuth backend ("No Garmin OAuth credentials") and fails the turn. The write
-- paths are now symmetric (delete token + remove connection together); this
-- one-time backfill heals the existing orphaned rows.
--
-- Scope: only 'oauth' and 'manual' connections require a backing token
-- ('manual' is how sciotte/sciotte_garmin register). 'synthetic' connections are
-- tokenless by design (data generated locally, some backfilled from
-- synthetic_activities without a token row) and MUST be spared.
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
