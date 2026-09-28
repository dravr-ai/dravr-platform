-- ABOUTME: Lets the outbound retry queue hold a send to a linked chat with no messaging session, guarded by expiry and connection state
-- ABOUTME: message_id becomes nullable; expires_at, reauth_tenant_id and reauth_provider bound when a queued reconnect notice may still go out
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A reconnect notice goes to every chat the athlete linked, and a linked chat
-- need not have a messaging session, so a failed send there has no message
-- row for the queue to reference: message_id becomes nullable.
--
-- expires_at: the instant after which the payload must not be sent (the
-- reconnect link it carries has expired). NULL for payloads with no expiring
-- content.
--
-- reauth_tenant_id / reauth_provider: set on a queued reconnect notice, naming
-- the connection (with the row's user_id) the notice is about. The worker
-- sends it only while that connection is still needs_reauth.

ALTER TABLE messaging_outbound_queue ALTER COLUMN message_id DROP NOT NULL;

ALTER TABLE messaging_outbound_queue ADD COLUMN IF NOT EXISTS expires_at TIMESTAMPTZ;
ALTER TABLE messaging_outbound_queue ADD COLUMN IF NOT EXISTS reauth_tenant_id TEXT;
ALTER TABLE messaging_outbound_queue ADD COLUMN IF NOT EXISTS reauth_provider TEXT;
