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
--
-- GROUP-bound sessions are deliberately left on the channel tenant. The runtime
-- only re-tenants DIRECT messages (resolve_linked_session: session_tenant =
-- is_direct_message ? user_tenant : channel_tenant); a shared coaching_group
-- must resolve to one row for members who may span tenants, so moving a
-- group session to one member's personal tenant would fragment the group. A
-- conversation with a non-NULL group_id is the durable marker of such a session.

-- 1. Direct-message sessions -> user's earliest tenant_users membership.
WITH user_tenant AS (
    -- Mirror the runtime's active_tenant_id resolution exactly
    -- (list_for_user().first(), backends/postgres/tenant.rs): the EARLIEST
    -- membership in an ACTIVE, OWNER-backed tenant. A bare tenant_users scan
    -- could pick an inactive or owner-less tenant the runtime will never resolve
    -- as active_tenant_id, recreating the invisible-session bug at a different
    -- wrong tenant. The t.id tie-break makes a joined_at tie deterministic.
    SELECT DISTINCT ON (tu.user_id) tu.user_id, t.id AS tenant_id
    FROM tenant_users tu
    JOIN tenants t ON t.id = tu.tenant_id AND t.is_active = true
    JOIN tenant_users owner ON owner.tenant_id = t.id AND owner.role = 'owner'
    ORDER BY tu.user_id, tu.joined_at ASC, t.id ASC
),
movable AS (
    SELECT s.id, ut.tenant_id AS new_tenant_id
    FROM messaging_sessions s
    JOIN user_tenant ut ON ut.user_id = s.user_id
    WHERE s.tenant_id <> ut.tenant_id
      AND NOT EXISTS (
          -- skip group-bound sessions (shared group must stay on one tenant)
          SELECT 1 FROM chat_conversations c
          WHERE c.id::text = s.pierre_conversation_id::text
            AND c.group_id IS NOT NULL
      )
      AND NOT EXISTS (
          SELECT 1 FROM messaging_sessions x
          WHERE x.tenant_id = ut.tenant_id
            AND x.channel_type = s.channel_type
            AND x.channel_user_id = s.channel_user_id
            AND COALESCE(x.channel_conversation_id, '') = COALESCE(s.channel_conversation_id, '')
            AND x.id <> s.id
      )
)
UPDATE messaging_sessions s
SET tenant_id = m.new_tenant_id
FROM movable m
WHERE s.id = m.id;

-- 2. Conversations follow their session's (now-corrected) tenant.
--    chat_conversations.tenant_id is varchar, so the uuid source is cast to text.
UPDATE chat_conversations c
SET tenant_id = s.tenant_id::text
FROM messaging_sessions s
WHERE s.pierre_conversation_id::text = c.id::text
  AND c.tenant_id <> s.tenant_id::text;

-- 3. Messages follow their session's tenant. messaging_messages.tenant_id is text;
--    messaging_messages.session_id is text while messaging_sessions.id is uuid.
--    The NOT EXISTS guard skips any message whose move would collide with the
--    (tenant_id, channel_message_id) unique index, so this one-shot re-tenant can
--    never abort the boot transaction (verified zero collisions on dev at write
--    time; the guard keeps that true for any future data).
UPDATE messaging_messages mm
SET tenant_id = s.tenant_id::text
FROM messaging_sessions s
WHERE mm.session_id = s.id::text
  AND mm.tenant_id <> s.tenant_id::text
  AND NOT EXISTS (
      SELECT 1 FROM messaging_messages other
      WHERE other.tenant_id = s.tenant_id::text
        AND other.channel_message_id = mm.channel_message_id
        AND other.id <> mm.id
  );
