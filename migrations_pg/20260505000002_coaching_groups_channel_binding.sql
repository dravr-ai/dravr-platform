-- ABOUTME: Bind coaching_groups to a channel chat (Telegram/Slack/Discord group id)
-- ABOUTME: Lets a messaging group chat auto-resolve to its coaching_groups row

-- Mirror of migrations/20260505000002 for Postgres. Same partial-unique-index
-- pattern; PG accepts WHERE clauses on UNIQUE INDEX.

ALTER TABLE coaching_groups ADD COLUMN IF NOT EXISTS channel_type TEXT;
ALTER TABLE coaching_groups ADD COLUMN IF NOT EXISTS channel_chat_id TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_coaching_groups_channel_chat
    ON coaching_groups(tenant_id, channel_type, channel_chat_id)
    WHERE channel_type IS NOT NULL AND channel_chat_id IS NOT NULL;
