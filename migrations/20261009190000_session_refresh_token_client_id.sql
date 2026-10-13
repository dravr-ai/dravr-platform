-- ABOUTME: Adds client_id to session_refresh_tokens: the first-party client (dravr-web, dravr-mobile) a session was signed in to
-- ABOUTME: Every JWT a refresh mints names that client in its client_id claim; NULL for a session no client signed in to

-- A session opened by redeeming an authorization code knows which of Dravr's
-- apps redeemed it, and its JWT says so in the RFC 9068 `client_id` claim. The
-- refresh token is what the device holds past that JWT's 24 hours, so it has to
-- carry the client too, or the session would stop naming its client at the
-- first refresh. Nullable and additive: a family opened before this column
-- existed keeps refreshing, as a session that names no client.
ALTER TABLE session_refresh_tokens
    ADD COLUMN client_id TEXT; -- idempotency-ok: SQLite has no ADD COLUMN IF NOT EXISTS; one-time additive column
