-- ABOUTME: Add channel linking tables for OAuth/deep-link account verification
-- ABOUTME: Maps authenticated Pierre users to their messaging channel identities
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Ephemeral link states for pending channel linking requests (10-minute TTL, one-time use)
CREATE TABLE IF NOT EXISTS messaging_link_states (
    id              TEXT    NOT NULL PRIMARY KEY,
    tenant_id       TEXT    NOT NULL,
    user_id         TEXT    NOT NULL,
    channel_type    TEXT    NOT NULL,  -- whatsapp, messenger, discord, slack, telegram
    code            TEXT    NOT NULL,  -- Verification code (cryptographically random)
    method          TEXT    NOT NULL,  -- deep_link or oauth
    used            INTEGER NOT NULL DEFAULT 0,
    expires_at      TEXT    NOT NULL,
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- Fast lookup by code for consumption
CREATE UNIQUE INDEX IF NOT EXISTS idx_messaging_link_states_code
    ON messaging_link_states(code);

-- Cleanup expired states
CREATE INDEX IF NOT EXISTS idx_messaging_link_states_expiry
    ON messaging_link_states(expires_at) WHERE used = 0;

-- Prevent duplicate pending links per user+channel
CREATE INDEX IF NOT EXISTS idx_messaging_link_states_user_channel
    ON messaging_link_states(tenant_id, user_id, channel_type) WHERE used = 0;

-- Permanent user-to-channel identity mappings
CREATE TABLE IF NOT EXISTS messaging_channel_links (
    id              TEXT    NOT NULL PRIMARY KEY,
    tenant_id       TEXT    NOT NULL,
    user_id         TEXT    NOT NULL,
    channel_type    TEXT    NOT NULL,  -- whatsapp, messenger, discord, slack, telegram
    channel_user_id TEXT    NOT NULL,  -- Platform-specific user identifier
    display_name    TEXT,              -- Human-readable name from the platform
    linked_at       TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(tenant_id, channel_type, channel_user_id)
);

-- Look up a link by user and channel type (for listing connected channels)
CREATE INDEX IF NOT EXISTS idx_messaging_channel_links_user
    ON messaging_channel_links(tenant_id, user_id);

-- Look up a link by channel identity (for inbound webhook user resolution)
CREATE INDEX IF NOT EXISTS idx_messaging_channel_links_channel_identity
    ON messaging_channel_links(tenant_id, channel_type, channel_user_id);
