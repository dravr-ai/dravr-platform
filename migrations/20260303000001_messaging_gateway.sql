-- ABOUTME: Create messaging gateway tables for multi-channel chat (WhatsApp, Messenger, Discord, Slack, Telegram)
-- ABOUTME: Channel configs, sessions, messages with idempotency, delivery receipts, and outbound retry queue
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Per-tenant channel configuration
CREATE TABLE IF NOT EXISTS messaging_channel_configs (
    id              TEXT    NOT NULL PRIMARY KEY,
    tenant_id       TEXT    NOT NULL,
    channel_type    TEXT    NOT NULL,  -- whatsapp, messenger, discord, slack, telegram
    api_key         TEXT,              -- Access token / Account SID
    api_secret      TEXT,              -- Auth token / API secret
    webhook_secret  TEXT,              -- Signing secret for webhook verification
    account_id      TEXT,              -- Platform-specific account ID (phone number ID, etc.)
    phone_number    TEXT,              -- WhatsApp/SMS phone number
    bot_token       TEXT,              -- Discord/Telegram bot token
    is_active       INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(tenant_id, channel_type)
);

CREATE INDEX IF NOT EXISTS idx_messaging_channel_configs_tenant
    ON messaging_channel_configs(tenant_id);

CREATE INDEX IF NOT EXISTS idx_messaging_channel_configs_active
    ON messaging_channel_configs(tenant_id, is_active) WHERE is_active = 1;

-- Active messaging sessions linking channel users to Pierre conversations
CREATE TABLE IF NOT EXISTS messaging_sessions (
    id                          TEXT    NOT NULL PRIMARY KEY,
    user_id                     TEXT    NOT NULL,
    tenant_id                   TEXT    NOT NULL,
    channel_type                TEXT    NOT NULL,
    channel_user_id             TEXT    NOT NULL,  -- Phone number, platform user ID, etc.
    channel_conversation_id     TEXT,              -- Thread/channel ID
    pierre_conversation_id      TEXT,              -- Pierre chat conversation ID
    last_message_at             TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    created_at                  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_messaging_sessions_tenant
    ON messaging_sessions(tenant_id);

-- Look up session by channel identity (for inbound message routing)
CREATE UNIQUE INDEX IF NOT EXISTS idx_messaging_sessions_channel_identity
    ON messaging_sessions(tenant_id, channel_type, channel_user_id);

CREATE INDEX IF NOT EXISTS idx_messaging_sessions_user
    ON messaging_sessions(user_id, tenant_id);

-- Message log with idempotency key (channel_message_id per tenant)
CREATE TABLE IF NOT EXISTS messaging_messages (
    id                  TEXT    NOT NULL PRIMARY KEY,
    tenant_id           TEXT    NOT NULL,
    session_id          TEXT    NOT NULL REFERENCES messaging_sessions(id),
    direction           TEXT    NOT NULL,  -- inbound, outbound
    channel_type        TEXT    NOT NULL,
    channel_message_id  TEXT    NOT NULL,  -- Channel-native message ID (idempotency key)
    sender_id           TEXT    NOT NULL,
    content_type        TEXT    NOT NULL,  -- text, media, location, card
    content_body        TEXT,              -- Text body or serialized content
    correlation_id      TEXT    NOT NULL,  -- Links inbound to processing to outbound
    raw_payload         TEXT,              -- Original webhook JSON for audit
    created_at          TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_messaging_messages_tenant
    ON messaging_messages(tenant_id);

CREATE INDEX IF NOT EXISTS idx_messaging_messages_session
    ON messaging_messages(session_id, created_at);

-- Idempotency: reject duplicate channel_message_id per tenant
CREATE UNIQUE INDEX IF NOT EXISTS idx_messaging_messages_idempotency
    ON messaging_messages(tenant_id, channel_message_id);

CREATE INDEX IF NOT EXISTS idx_messaging_messages_correlation
    ON messaging_messages(correlation_id);

-- Delivery receipts tracking outbound message lifecycle
CREATE TABLE IF NOT EXISTS messaging_delivery_receipts (
    id                  TEXT    NOT NULL PRIMARY KEY,
    tenant_id           TEXT    NOT NULL,
    message_id          TEXT    NOT NULL REFERENCES messaging_messages(id),
    channel_message_id  TEXT,              -- Channel-assigned message ID
    status              TEXT    NOT NULL,  -- pending, sent, delivered, read, failed, dlq
    created_at          TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_messaging_delivery_receipts_message
    ON messaging_delivery_receipts(message_id);

CREATE INDEX IF NOT EXISTS idx_messaging_delivery_receipts_tenant
    ON messaging_delivery_receipts(tenant_id);

-- Outbound message queue for retry tracking with dead-letter support
CREATE TABLE IF NOT EXISTS messaging_outbound_queue (
    id              TEXT    NOT NULL PRIMARY KEY,
    message_id      TEXT    NOT NULL REFERENCES messaging_messages(id),
    tenant_id       TEXT    NOT NULL,
    channel_type    TEXT    NOT NULL,
    payload         TEXT    NOT NULL,  -- Serialized outbound payload JSON
    status          TEXT    NOT NULL DEFAULT 'pending',  -- pending, sent, retrying:N, dlq
    attempt_count   INTEGER NOT NULL DEFAULT 0,
    next_retry_at   TEXT,              -- Scheduled time for next retry
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_messaging_outbound_queue_tenant
    ON messaging_outbound_queue(tenant_id);

-- Poll for retryable messages ordered by scheduled time
CREATE INDEX IF NOT EXISTS idx_messaging_outbound_queue_retry
    ON messaging_outbound_queue(status, next_retry_at)
    WHERE status LIKE 'pending' OR status LIKE 'retrying:%';

CREATE INDEX IF NOT EXISTS idx_messaging_outbound_queue_message
    ON messaging_outbound_queue(message_id);
