-- ABOUTME: Adds channel_user_id and sender_name columns to messaging_link_states for webhook-initiated linking
-- ABOUTME: Fixes missing columns when base migration was applied before channel-initiated flow was added

-- Add channel identity columns for webhook-initiated linking flows
-- These allow the server to store who sent the message that triggered linking
ALTER TABLE messaging_link_states ADD COLUMN IF NOT EXISTS channel_user_id TEXT;
ALTER TABLE messaging_link_states ADD COLUMN IF NOT EXISTS sender_name TEXT;
