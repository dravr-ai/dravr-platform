-- ABOUTME: Records on each messaging session whether its chat is a direct message or a room (SQLite)
-- ABOUTME: Adds messaging_sessions.is_direct_message: what the classifier reads to tell a DM's thread from a group chat's

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- is_direct_message: TRUE for a session of a direct message, FALSE for one of
-- a group chat, NULL for a session no turn has stamped since this column
-- arrived. The ingress stamps it on every turn, from the channel's own report
-- of the chat (carnet#1422). A row a group chat's session writes into is never
-- served as private, whatever else it carries; a thread a DM's session writes
-- into stays a direct message. NULL is read as a room on every thread, the
-- owner's personal one included: a group chat's row may have been designated
-- personal before the adoption read the sessions.
ALTER TABLE messaging_sessions ADD COLUMN is_direct_message BOOLEAN; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- The sessions the stored rows already tell apart, by the pinned messaging
-- adapter's own rules (dravr-canot v0.14.0). A session writing into a
-- group-bound conversation is a group chat's. Telegram: only a chat whose
-- type is "private" is a DM, and its id is its user's id, a positive number;
-- a group's, a supergroup's and a channel's ids are negative. Slack: a
-- channel id starting with `D` is a DM, `C` a public channel and `G` a
-- private channel or a group DM. WhatsApp's Cloud API and Messenger deliver
-- only one-to-one messages, so every session of theirs is a DM. Discord
-- tells a DM by the absence of a guild id, which no stored column keeps, so
-- its sessions wait for their next turn, as does any session these rules do
-- not decide.
UPDATE messaging_sessions SET is_direct_message = FALSE
WHERE EXISTS (
    SELECT 1 FROM chat_conversations c
    WHERE c.id = messaging_sessions.pierre_conversation_id AND c.group_id IS NOT NULL
);

UPDATE messaging_sessions SET is_direct_message = FALSE
WHERE is_direct_message IS NULL
  AND ((channel_type = 'telegram' AND channel_conversation_id LIKE '-%')
       OR (channel_type = 'slack' AND channel_conversation_id IS NOT NULL
           AND channel_conversation_id NOT LIKE 'D%'));

UPDATE messaging_sessions SET is_direct_message = TRUE
WHERE is_direct_message IS NULL
  AND ((channel_type = 'telegram' AND channel_conversation_id = channel_user_id
        AND channel_conversation_id NOT LIKE '-%')
       OR (channel_type = 'slack' AND channel_conversation_id LIKE 'D%')
       OR channel_type IN ('whatsapp', 'messenger'));
