-- ABOUTME: Lets the outbound retry queue hold a send to a linked chat with no messaging session, guarded by expiry and connection state
-- ABOUTME: message_id becomes nullable; expires_at, reauth_tenant_id and reauth_provider bound when a queued reconnect notice may still go out
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A reconnect notice goes to every chat the athlete linked, and a linked chat
-- need not have a messaging session: nothing in the transcript tables names
-- it, so a failed send there has no message row for the queue to reference.
-- message_id becomes nullable so such a send is queued on its own, keyed by
-- the queue row's tenant (the bot that holds the chat), channel and payload.
--
-- expires_at: the instant after which the payload must not be sent — the
-- reconnect link it carries has expired by then. NULL for payloads with no
-- expiring content.
--
-- reauth_tenant_id / reauth_provider: set on a queued reconnect notice, naming
-- the connection (with the row's user_id) the notice is about. The worker
-- sends it only while that connection is still needs_reauth.
--
-- SQLite cannot drop a NOT NULL constraint in place, so the table is rebuilt.

CREATE TABLE IF NOT EXISTS messaging_outbound_queue_new (
    id                  TEXT    NOT NULL PRIMARY KEY,
    message_id          TEXT    REFERENCES messaging_messages(id),
    tenant_id           TEXT    NOT NULL,
    channel_type        TEXT    NOT NULL,
    payload             TEXT    NOT NULL,
    status              TEXT    NOT NULL DEFAULT 'pending',
    attempt_count       INTEGER NOT NULL DEFAULT 0,
    next_retry_at       TEXT,
    created_at          TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at          TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    user_id             TEXT,
    expires_at          TEXT,
    reauth_tenant_id    TEXT,
    reauth_provider     TEXT
);

INSERT INTO messaging_outbound_queue_new
    (id, message_id, tenant_id, channel_type, payload, status, attempt_count,
     next_retry_at, created_at, updated_at, user_id)
SELECT id, message_id, tenant_id, channel_type, payload, status, attempt_count,
       next_retry_at, created_at, updated_at, user_id
FROM messaging_outbound_queue;

DROP TABLE messaging_outbound_queue; -- idempotency-ok: table rebuild, the copy above only exists once the old table does
ALTER TABLE messaging_outbound_queue_new RENAME TO messaging_outbound_queue;

CREATE INDEX IF NOT EXISTS idx_messaging_outbound_queue_tenant
    ON messaging_outbound_queue(tenant_id);

CREATE INDEX IF NOT EXISTS idx_messaging_outbound_queue_retry
    ON messaging_outbound_queue(status, next_retry_at)
    WHERE status LIKE 'pending' OR status LIKE 'retrying:%';

CREATE INDEX IF NOT EXISTS idx_messaging_outbound_queue_message
    ON messaging_outbound_queue(message_id);

CREATE INDEX IF NOT EXISTS idx_messaging_outbound_queue_user
    ON messaging_outbound_queue(user_id);
