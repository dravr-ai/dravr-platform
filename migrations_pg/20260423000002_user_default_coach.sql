-- ABOUTME: Adds default_coach_id column to users table (PostgreSQL)
-- ABOUTME: Per-user coach selection set via /coach select in DM contexts; FK → coaches(id) ON DELETE SET NULL

ALTER TABLE users ADD COLUMN default_coach_id TEXT REFERENCES coaches(id) ON DELETE SET NULL;
