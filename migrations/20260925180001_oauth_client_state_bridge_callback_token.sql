-- ABOUTME: Stores the per-flow token of the local bridge listener that started a provider OAuth flow
-- ABOUTME: The success notification presents it, so the listener can tell its own flow's server from anyone else

-- The SDK bridge starts a provider OAuth flow with the token its local callback
-- listener demands of every provider-token POST. It is kept with the flow's
-- state until the callback redeems it, then presented on the one notification
-- that flow sends. NULL = no bridge started the flow, so nothing is notified.
ALTER TABLE oauth_client_states ADD COLUMN bridge_callback_token TEXT; -- idempotency-ok: SQLite has no ADD COLUMN IF NOT EXISTS; brand-new column, PG mirror uses IF NOT EXISTS
