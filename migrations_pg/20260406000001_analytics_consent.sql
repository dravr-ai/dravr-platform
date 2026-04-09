-- ABOUTME: Adds analytics consent columns to users table (PostgreSQL version)
-- ABOUTME: Default opt-out (false); users must explicitly opt in via Settings > Privacy & Data

ALTER TABLE users ADD COLUMN analytics_consent BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE users ADD COLUMN analytics_consent_at TIMESTAMPTZ;
