-- ABOUTME: Messaging provider connections and channel bindings schema migration
-- ABOUTME: Enables bidirectional chat bridging between external platforms (Slack, Discord) and Dravr AI

-- Messaging provider connections (workspace-level, per-tenant)
-- Each record represents a connected workspace/server from an external messaging platform.
-- Credentials (bot_token, signing_secret) are stored encrypted at rest.
CREATE TABLE IF NOT EXISTS messaging_connections (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    provider TEXT NOT NULL,             -- 'slack', 'discord', 'teams', etc.
    team_id TEXT NOT NULL,              -- workspace/server ID from the provider
    team_name TEXT,                     -- human-readable workspace name
    bot_token TEXT NOT NULL,            -- encrypted bot token for API calls
    signing_secret TEXT NOT NULL,       -- encrypted webhook signing secret
    created_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(tenant_id, provider, team_id)
);

-- Channel bindings: links an external provider channel to a Dravr conversation
-- When active, messages in the external channel are forwarded to the Dravr AI coach,
-- and AI responses are posted back to the external channel.
CREATE TABLE IF NOT EXISTS channel_bindings (
    id TEXT PRIMARY KEY,
    messaging_connection_id TEXT NOT NULL REFERENCES messaging_connections(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,           -- provider-specific channel identifier
    channel_name TEXT,                  -- human-readable channel name
    conversation_id TEXT NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(messaging_connection_id, channel_id)
);

-- Indexes for messaging connections
CREATE INDEX IF NOT EXISTS idx_messaging_connections_tenant ON messaging_connections(tenant_id);
CREATE INDEX IF NOT EXISTS idx_messaging_connections_provider_team ON messaging_connections(provider, team_id);

-- Indexes for channel bindings
CREATE INDEX IF NOT EXISTS idx_channel_bindings_channel ON channel_bindings(channel_id, messaging_connection_id);
CREATE INDEX IF NOT EXISTS idx_channel_bindings_conversation ON channel_bindings(conversation_id);
CREATE INDEX IF NOT EXISTS idx_channel_bindings_tenant ON channel_bindings(tenant_id);
CREATE INDEX IF NOT EXISTS idx_channel_bindings_active ON channel_bindings(active, messaging_connection_id);
