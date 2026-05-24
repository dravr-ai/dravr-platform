-- ABOUTME: Track the most recent time a provider was actually used (chat tool reads, REST fetches)
-- ABOUTME: Enables per-user "last connected provider" resolution when LLM tool calls omit the provider arg

-- Distinct from connected_at (when the connection was first established) and from
-- user_oauth_tokens.last_sync (when the background sync orchestrator ran). last_used_at
-- tracks the read path that serves the user's actual data requests — the field the
-- chat-pipeline resolver picks the active backend from.

ALTER TABLE provider_connections ADD COLUMN last_used_at TEXT;

-- Resolver lookup: most-recently-used connection for a user wins.
--
-- The query's ORDER BY `last_used_at DESC NULLS LAST` keeps freshly-added connections
-- (no last_used_at yet) ranked after connections with a real touch timestamp. The
-- index leaves the NULL-ordering to the planner — SQLite's DESC default for NULLs is
-- "first" but the query's explicit `NULLS LAST` overrides that at scan time. We omit
-- `NULLS LAST` from the index definition because the bundled SQLite shipped with
-- sqlx-sqlite (and several Linux distros) doesn't accept it in CREATE INDEX clauses
-- even when it's accepted in ORDER BY.
CREATE INDEX IF NOT EXISTS idx_provider_connections_last_used
    ON provider_connections(user_id, tenant_id, last_used_at DESC, connected_at DESC);
