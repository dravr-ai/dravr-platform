-- ABOUTME: Adds archived_at to chat_conversations: the instant /reset retired a thread, NULL while it is active
-- ABOUTME: The max_active_conversations quota counts only rows where archived_at IS NULL

-- A retired thread stays readable in the list; it no longer occupies one of
-- the owner's active-conversation slots.
ALTER TABLE chat_conversations
    ADD COLUMN archived_at TEXT; -- idempotency-ok: SQLite has no ADD COLUMN IF NOT EXISTS; one-time additive column
