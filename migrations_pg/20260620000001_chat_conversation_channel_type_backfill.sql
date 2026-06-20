-- ABOUTME: Backfills chat_conversations.channel_type for pre-existing messaging conversations
-- ABOUTME: The column exists (NOT NULL DEFAULT 'web', migration 20260424000001); ingress now stamps it, this repairs old rows

-- Messaging conversations created before messaging-ingress started stamping
-- channel_type are still 'web'. Recover their real origin from the
-- "Messaging: <channel>" title the gateway wrote at creation so the durable
-- channel badge (channel_type-first, title fallback) works for them too.
UPDATE chat_conversations
SET channel_type = lower(trim(substr(title, 12)))
WHERE channel_type = 'web' AND title LIKE 'Messaging: %';
