-- ABOUTME: Adds default_coach_id column to users table (SQLite)
-- ABOUTME: Per-user coach selection set via /coach select in DM contexts; nullable, no FK constraint (SQLite FK add-column limitation)

ALTER TABLE users ADD COLUMN default_coach_id TEXT REFERENCES coaches(id) ON DELETE SET NULL;
