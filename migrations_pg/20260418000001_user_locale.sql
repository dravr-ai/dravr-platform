-- ABOUTME: Adds locale column to users table (PostgreSQL)
-- ABOUTME: BCP-47 short code ('fr', 'en', 'es', 'de', 'pt'); default 'fr' matches DEFAULT_LOCALE in messaging_strings

ALTER TABLE users ADD COLUMN locale TEXT NOT NULL DEFAULT 'fr';
