-- ABOUTME: Adds OTP in-chat linking columns to messaging_link_states
-- ABOUTME: Enables email verification flow for account linking without web app

ALTER TABLE messaging_link_states ADD COLUMN IF NOT EXISTS otp_step TEXT;
ALTER TABLE messaging_link_states ADD COLUMN IF NOT EXISTS email TEXT;
ALTER TABLE messaging_link_states ADD COLUMN IF NOT EXISTS otp_hash TEXT;
ALTER TABLE messaging_link_states ADD COLUMN IF NOT EXISTS otp_attempts INTEGER NOT NULL DEFAULT 0;

CREATE UNIQUE INDEX IF NOT EXISTS idx_messaging_link_states_active_otp
    ON messaging_link_states(tenant_id, channel_type, channel_user_id)
    WHERE otp_step IS NOT NULL AND used = FALSE;
