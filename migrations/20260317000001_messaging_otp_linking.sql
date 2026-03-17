-- ABOUTME: Adds OTP in-chat linking columns to messaging_link_states
-- ABOUTME: Enables email verification flow for account linking without web app

ALTER TABLE messaging_link_states ADD COLUMN otp_step TEXT;
ALTER TABLE messaging_link_states ADD COLUMN email TEXT;
ALTER TABLE messaging_link_states ADD COLUMN otp_hash TEXT;
ALTER TABLE messaging_link_states ADD COLUMN otp_attempts INTEGER NOT NULL DEFAULT 0;
