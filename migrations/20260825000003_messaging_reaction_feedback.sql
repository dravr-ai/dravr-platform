-- ABOUTME: Links outbound messaging rows to the assistant chat message they delivered (SQLite)
-- ABOUTME: Lets an emoji reaction on a channel message resolve to chat_messages.id for the shared feedback write

-- A reaction webhook carries only the channel's own message id (Telegram
-- message_id, Slack ts, Discord snowflake). Feedback is keyed on
-- chat_messages.id. The outbound persist is the one moment both ids are in
-- hand, so it stamps the assistant message id here. NULL for inbound rows and
-- for outbound rows that deliver no assistant coaching reply (cards, intake
-- questions, error apologies) — those are not ratable messages.
ALTER TABLE messaging_messages ADD COLUMN chat_message_id TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- Reaction resolution looks the sent message up by channel identity, not by
-- tenant (the webhook authenticates as the bot's tenant while DM rows live
-- under the athlete's own). Partial: only rows that can receive feedback.
CREATE INDEX IF NOT EXISTS idx_messaging_messages_reaction_target
    ON messaging_messages(channel_type, channel_message_id)
    WHERE chat_message_id IS NOT NULL;
