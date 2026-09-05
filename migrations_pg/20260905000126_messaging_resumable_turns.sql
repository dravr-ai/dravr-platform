-- ABOUTME: Durable hand-off for a messaging turn the shutdown drain interrupted, so another instance answers it
-- ABOUTME: One row per drained inbound message, leased by the instance re-running it, deleted once answered
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A messaging turn runs after its webhook answered 200, so Cloud Run reads the
-- instance as idle and may drain it mid-turn (registre#126). The drain used to
-- close the athlete's placeholder with an apology; this table is what lets it
-- hand the turn to the next instance instead. Everything a fresh dispatch needs
-- is copied here at the moment the drain signal fires — the inbound message
-- itself is already in messaging_messages, but that row carries none of the
-- resolved tenants, the locale, the thread, or the placeholder the reply must
-- be edited into.
--
-- Not a column on messaging_messages: that table is shared by both directions
-- and by reactions, and a resume needs three tenants plus a lease, which is a
-- row of its own. tenant_id is the SESSION tenant (owner of the conversation
-- and of the inbound row); the channel and user tenants ride alongside because
-- the re-dispatch needs all three exactly as the webhook resolved them. TEXT
-- ids throughout, matching every other messaging_* table on this backend.
--
-- (tenant_id, channel_type, channel_message_id) is unique so a turn is recorded
-- once per inbound message, whichever instance drains it. Timestamps are unix
-- milliseconds so the lease comparison is one integer test on both backends.
CREATE TABLE IF NOT EXISTS messaging_resumable_turns (
    id                      TEXT    NOT NULL PRIMARY KEY,
    tenant_id               TEXT    NOT NULL,  -- session tenant: owns the conversation and the inbound row
    channel_tenant_id       TEXT    NOT NULL,  -- bot/channel-owner tenant: channel config, link, outbound send
    user_tenant_id          TEXT    NOT NULL,  -- athlete's own tenant: tools, credentials, usage counters
    session_id              TEXT    NOT NULL,
    conversation            TEXT    NOT NULL,  -- Pierre conversation id
    user_id                 TEXT    NOT NULL,
    channel_type            TEXT    NOT NULL,  -- channel slug ("telegram")
    sender_id               TEXT    NOT NULL,
    conversation_id         TEXT,              -- channel-native chat id
    channel_message_id      TEXT    NOT NULL,  -- inbound message id: the idempotency key
    thread_id               TEXT,
    text_content            TEXT    NOT NULL,  -- the sanitized text the LLM was given
    is_group_chat           BOOLEAN NOT NULL,
    locale                  TEXT    NOT NULL,
    turn_id                 TEXT    NOT NULL,
    placeholder_message_id  TEXT,              -- the status placeholder the resumed reply edits
    attempts                BIGINT  NOT NULL DEFAULT 1,  -- runs started so far, the drained one included; BIGINT so it decodes as the i64 the row carries
    created_at_ms           BIGINT  NOT NULL,
    leased_by               TEXT,
    leased_until_ms         BIGINT
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_messaging_resumable_turns_inbound
    ON messaging_resumable_turns(tenant_id, channel_type, channel_message_id);

CREATE INDEX IF NOT EXISTS idx_messaging_resumable_turns_lease
    ON messaging_resumable_turns(leased_until_ms, created_at_ms);
