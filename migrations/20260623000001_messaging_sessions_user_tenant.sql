-- ABOUTME: One-time re-tenant of messaging sessions (+ their conversations and messages) off the
-- ABOUTME: bot/channel-owner tenant onto each user's OWN tenant, so chat data aligns with their activity cache
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Messaging sessions were historically stored under the tenant the inbound
-- webhook carried — the BOT owner's tenant (e.g. an admin-owned Telegram bot),
-- not the user's own personal-workspace tenant. Tool execution, the activity
-- cache, and the backfill-completion push all key off the user's
-- active_tenant_id (= list_for_user().first(), the earliest tenant_users
-- membership), so a session under the bot tenant is invisible to the push:
-- get_session_by_pierre_conversation_id looks under the user tenant and finds
-- nothing. The companion code change makes resolve_linked_session store every
-- new session under the user tenant; this migration consolidates the EXISTING
-- split rows so history and any in-flight backfill push line up under one
-- tenant per user.
--
-- The target tenant is resolved exactly the way the runtime resolves
-- active_tenant_id: the user's earliest tenant_users membership. Users with no
-- membership are left untouched (the runtime falls back to the webhook tenant
-- for them too). A NOT EXISTS guard skips any session that would collide with an
-- already-correct session for the same (tenant, channel, channel_user, chat),
-- leaving the stray row inert rather than violating the per-chat uniqueness.
-- SQLite stores every id/tenant as TEXT, so no casts are needed here.
--
-- GROUP-bound sessions are deliberately left on the channel tenant. The runtime
-- only re-tenants DIRECT messages (resolve_linked_session: session_tenant =
-- is_direct_message ? user_tenant : channel_tenant); a shared coaching_group
-- must resolve to one row for members who may span tenants, so moving a
-- group session to one member's personal tenant would fragment the group. A
-- conversation with a non-NULL group_id is the durable marker of such a session.

-- 1. Direct-message sessions -> user's resolved active tenant. The CTE mirrors
--    the runtime active_tenant_id resolution (list_for_user().first()): the
--    earliest membership in an ACTIVE, OWNER-backed tenant (joined_at then
--    tenant_id for a deterministic tie-break). A bare tenant_users scan could
--    pick an inactive/owner-less tenant the runtime never resolves.
WITH user_tenant AS (
    SELECT tu.user_id AS uid, tu.tenant_id AS tid,
           ROW_NUMBER() OVER (
               PARTITION BY tu.user_id
               ORDER BY tu.joined_at ASC, tu.tenant_id ASC
           ) AS rn
    FROM tenant_users tu
    JOIN tenants t ON t.id = tu.tenant_id AND t.is_active = 1
    WHERE EXISTS (
        SELECT 1 FROM tenant_users o
        WHERE o.tenant_id = tu.tenant_id AND o.role = 'owner'
    )
)
UPDATE messaging_sessions
SET tenant_id = (
    SELECT tid FROM user_tenant
    WHERE uid = messaging_sessions.user_id AND rn = 1
)
WHERE EXISTS (
    SELECT 1 FROM user_tenant
    WHERE uid = messaging_sessions.user_id AND rn = 1
)
AND tenant_id <> (
    SELECT tid FROM user_tenant
    WHERE uid = messaging_sessions.user_id AND rn = 1
)
AND NOT EXISTS (
    -- skip group-bound sessions (shared group must stay on one tenant)
    SELECT 1 FROM chat_conversations c
    WHERE c.id = messaging_sessions.pierre_conversation_id
      AND c.group_id IS NOT NULL
)
AND NOT EXISTS (
    SELECT 1 FROM messaging_sessions x
    WHERE x.tenant_id = (
        SELECT tid FROM user_tenant
        WHERE uid = messaging_sessions.user_id AND rn = 1
    )
    AND x.channel_type = messaging_sessions.channel_type
    AND x.channel_user_id = messaging_sessions.channel_user_id
    AND COALESCE(x.channel_conversation_id, '') = COALESCE(messaging_sessions.channel_conversation_id, '')
    AND x.id <> messaging_sessions.id
);

-- 2. Conversations follow their session's (now-corrected) tenant.
UPDATE chat_conversations
SET tenant_id = (
    SELECT s.tenant_id FROM messaging_sessions s
    WHERE s.pierre_conversation_id = chat_conversations.id
)
WHERE EXISTS (
    SELECT 1 FROM messaging_sessions s
    WHERE s.pierre_conversation_id = chat_conversations.id
      AND s.tenant_id <> chat_conversations.tenant_id
);

-- 3. Messages follow their session's tenant. The NOT EXISTS guard skips any
--    message whose move would collide with the (tenant_id, channel_message_id)
--    unique index, so this one-shot re-tenant can never abort the boot
--    transaction (verified zero collisions on dev at write time).
UPDATE messaging_messages
SET tenant_id = (
    SELECT s.tenant_id FROM messaging_sessions s
    WHERE s.id = messaging_messages.session_id
)
WHERE EXISTS (
    SELECT 1 FROM messaging_sessions s
    WHERE s.id = messaging_messages.session_id
      AND s.tenant_id <> messaging_messages.tenant_id
      AND NOT EXISTS (
          SELECT 1 FROM messaging_messages other
          WHERE other.tenant_id = s.tenant_id
            AND other.channel_message_id = messaging_messages.channel_message_id
            AND other.id <> messaging_messages.id
      )
);
