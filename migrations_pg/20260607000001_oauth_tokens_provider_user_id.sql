-- ABOUTME: Adds provider_user_id to user_oauth_tokens for API-key providers (e.g. Intervals.icu)
-- ABOUTME: Stores the user's provider-side id (athlete_id) in plaintext; NULL for OAuth bearer providers

ALTER TABLE user_oauth_tokens ADD COLUMN IF NOT EXISTS provider_user_id TEXT;
