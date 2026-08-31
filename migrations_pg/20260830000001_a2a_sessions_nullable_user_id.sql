-- ABOUTME: Drops NOT NULL constraint on user_id in a2a_sessions for client-keyed sessions
-- ABOUTME: A2ARepository::create_session takes an optional user; the SQLite schema has always allowed NULL here

ALTER TABLE a2a_sessions ALTER COLUMN user_id DROP NOT NULL;
