-- ABOUTME: Drops NOT NULL constraint on user_id in messaging_link_states for webhook-initiated linking
-- ABOUTME: Webhook-initiated flows create link states before user authenticates, requiring nullable user_id

ALTER TABLE messaging_link_states ALTER COLUMN user_id DROP NOT NULL;
