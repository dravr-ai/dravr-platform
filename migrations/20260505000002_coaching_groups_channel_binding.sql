-- ABOUTME: Bind coaching_groups to a channel chat (Telegram/Slack/Discord group id)
-- ABOUTME: Lets a messaging group chat auto-resolve to its coaching_groups row

-- A coaching_group can be created via REST (web/mobile) OR via the messaging
-- ingress when the bot first sees a non-DM chat (Telegram group, Slack channel,
-- Discord channel). For the latter case we need to look up "is there already a
-- coaching_group bound to (channel_type='telegram', channel_chat_id='-100123')?"
-- on every inbound message — both columns NULL for REST-created groups.

ALTER TABLE coaching_groups ADD COLUMN channel_type TEXT;
ALTER TABLE coaching_groups ADD COLUMN channel_chat_id TEXT;

-- Partial unique index: only enforces uniqueness when both are set, leaving
-- legacy REST-created groups (both NULL) free to coexist.
CREATE UNIQUE INDEX IF NOT EXISTS idx_coaching_groups_channel_chat
    ON coaching_groups(tenant_id, channel_type, channel_chat_id)
    WHERE channel_type IS NOT NULL AND channel_chat_id IS NOT NULL;
