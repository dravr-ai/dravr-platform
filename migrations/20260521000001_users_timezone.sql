-- ABOUTME: Adds timezone column to users table (SQLite)
-- ABOUTME: IANA TZ database name ('America/Toronto', 'Europe/Paris', ...); NULL = client never reported, server reads as UTC

ALTER TABLE users ADD COLUMN timezone TEXT;
