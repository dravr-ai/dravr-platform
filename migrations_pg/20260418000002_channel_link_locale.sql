-- ABOUTME: Adds optional per-channel locale override to messaging_channel_links (PostgreSQL)
-- ABOUTME: NULL means inherit from users.locale; explicit value wins (e.g. Telegram in EN while users.locale='fr')

ALTER TABLE messaging_channel_links ADD COLUMN locale TEXT;
