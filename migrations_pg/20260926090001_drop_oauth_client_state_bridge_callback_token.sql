-- ABOUTME: Drops oauth_client_states.bridge_callback_token; Dravr no longer posts a completed flow to a local bridge listener (PostgreSQL version)
-- ABOUTME: The SDK bridge polls get_connection_status for the provider instead, so no flow carries a listener token

-- A provider OAuth flow completes on the server and nowhere else: the SDK
-- bridge learns the outcome by asking for the provider's connection status,
-- which works whatever host the bridge runs on. With no localhost
-- notification left to authenticate, the flow's state carries no bridge token.
ALTER TABLE oauth_client_states DROP COLUMN IF EXISTS bridge_callback_token;
