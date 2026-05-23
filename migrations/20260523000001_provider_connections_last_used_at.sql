-- ABOUTME: Track the most recent time a provider was actually used (chat tool reads, REST fetches)
-- ABOUTME: Enables per-user "last connected provider" resolution when LLM tool calls omit the provider arg

-- Distinct from connected_at (when the connection was first established) and from
-- user_oauth_tokens.last_sync (when the background sync orchestrator ran). last_used_at
-- tracks the read path that serves the user's actual data requests — the field the
-- chat-pipeline resolver picks the active backend from.

ALTER TABLE provider_connections ADD COLUMN last_used_at TEXT;

-- Resolver lookup: most-recently-used connection for a user wins. The composite index
-- with NULLS LAST keeps freshly-added connections (no last_used_at yet) ranked after
-- connections with a real touch timestamp.
CREATE INDEX IF NOT EXISTS idx_provider_connections_last_used
    ON provider_connections(user_id, tenant_id, last_used_at DESC NULLS LAST, connected_at DESC);
