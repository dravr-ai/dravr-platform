-- ABOUTME: Adds coaching_persona + manages_roster columns to users table (PostgreSQL)
-- ABOUTME: Persona controls coach output format/cadence; manages_roster gates Coach-tier roster UI

ALTER TABLE users ADD COLUMN coaching_persona TEXT NOT NULL DEFAULT 'casual';
ALTER TABLE users ADD COLUMN manages_roster BOOLEAN NOT NULL DEFAULT FALSE;
