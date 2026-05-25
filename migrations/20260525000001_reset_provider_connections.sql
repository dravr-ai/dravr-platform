-- ABOUTME: One-shot reset of all provider connections to force re-onboarding to Strava/Garmin/Whoop only
-- ABOUTME: Wipes provider_connections + user_oauth_tokens but preserves user_oauth_app_credentials (BYO Whoop app)

DELETE FROM provider_connections;
DELETE FROM user_oauth_tokens;
