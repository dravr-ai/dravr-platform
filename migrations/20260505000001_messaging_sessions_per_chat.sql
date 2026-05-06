-- ABOUTME: Widen messaging_sessions UNIQUE constraint to include channel_conversation_id
-- ABOUTME: Lets a single channel user have distinct Pierre sessions per chat (DM vs group)

-- Context: today the lookup `get_session_by_channel_identity(tenant, channel, user_id)`
-- finds at most one session per (tenant, channel_type, channel_user_id). When the
-- same Telegram/Slack/Discord user messages the bot in both a private chat (DM)
-- AND a group chat, both collapse into one Pierre conversation, so the LLM has no
-- way to know which physical chat the message came from. Widening the unique
-- constraint to include channel_conversation_id splits them: one row per
-- (tenant, channel, user, chat). DMs (where the platform delivers a non-null
-- chat id equal to or paired with the user id) and group chats coexist.
--
-- channel_conversation_id is NULL for legacy rows. The COALESCE expression below
-- treats NULL as the empty sentinel '' so two rows with NULL would collide —
-- legacy rows are pre-dated and there is at most one per (tenant, channel, user).

DROP INDEX IF EXISTS idx_messaging_sessions_channel_identity;

CREATE UNIQUE INDEX IF NOT EXISTS idx_messaging_sessions_channel_chat
    ON messaging_sessions(
        tenant_id,
        channel_type,
        channel_user_id,
        COALESCE(channel_conversation_id, '')
    );
