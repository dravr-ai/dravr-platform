-- ABOUTME: Adds nullable theme column to users table (PostgreSQL)
-- ABOUTME: 'light'/'dark' pins a scheme across devices; NULL = follow the system (server-side chart renders read NULL as dark)

ALTER TABLE users ADD COLUMN IF NOT EXISTS theme TEXT CHECK (theme IN ('light', 'dark'));
