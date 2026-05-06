-- ABOUTME: Widen messaging_sessions UNIQUE constraint to include channel_conversation_id
-- ABOUTME: Lets a single channel user have distinct Pierre sessions per chat (DM vs group)

-- Mirror of migrations/20260505000001 for Postgres. See the SQLite copy for
-- the rationale; PG honors the same COALESCE-as-sentinel pattern.

DROP INDEX IF EXISTS idx_messaging_sessions_channel_identity;

CREATE UNIQUE INDEX IF NOT EXISTS idx_messaging_sessions_channel_chat
    ON messaging_sessions(
        tenant_id,
        channel_type,
        channel_user_id,
        COALESCE(channel_conversation_id, '')
    );
