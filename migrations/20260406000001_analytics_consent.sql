-- ABOUTME: Adds analytics consent columns to users table
-- ABOUTME: Default opt-out (0); users must explicitly opt in via Settings > Privacy & Data

ALTER TABLE users ADD COLUMN analytics_consent INTEGER NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN analytics_consent_at TEXT;
