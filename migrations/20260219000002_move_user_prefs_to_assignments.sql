-- ABOUTME: Move user-preference fields from coaches to coach_assignments table
-- ABOUTME: Part of Coach struct decomposition (DRAVR-593)
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Add user-preference columns to coach_assignments
ALTER TABLE coach_assignments ADD COLUMN is_favorite INTEGER NOT NULL DEFAULT 0;
ALTER TABLE coach_assignments ADD COLUMN is_active INTEGER NOT NULL DEFAULT 0;
ALTER TABLE coach_assignments ADD COLUMN use_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE coach_assignments ADD COLUMN last_used_at TEXT;

-- Create self-assignment rows for personal coaches (user-owned, non-system)
-- so their existing preference values are preserved
INSERT OR IGNORE INTO coach_assignments (id, coach_id, user_id, assigned_by, created_at)
SELECT
    lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' ||
          substr(hex(randomblob(2)),2) || '-' ||
          substr('89ab', abs(random()) % 4 + 1, 1) ||
          substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))) as id,
    c.id as coach_id,
    c.user_id as user_id,
    c.user_id as assigned_by,
    c.created_at
FROM coaches c
WHERE c.is_system = 0;

-- Copy preference values from coaches to their assignment rows
UPDATE coach_assignments
SET
    is_favorite = (SELECT c.is_favorite FROM coaches c WHERE c.id = coach_assignments.coach_id AND c.user_id = coach_assignments.user_id),
    is_active = (SELECT c.is_active FROM coaches c WHERE c.id = coach_assignments.coach_id AND c.user_id = coach_assignments.user_id),
    use_count = (SELECT c.use_count FROM coaches c WHERE c.id = coach_assignments.coach_id AND c.user_id = coach_assignments.user_id),
    last_used_at = (SELECT c.last_used_at FROM coaches c WHERE c.id = coach_assignments.coach_id AND c.user_id = coach_assignments.user_id)
WHERE EXISTS (
    SELECT 1 FROM coaches c
    WHERE c.id = coach_assignments.coach_id AND c.user_id = coach_assignments.user_id
);

-- Add index for active coach lookup (only one active per user)
CREATE INDEX IF NOT EXISTS idx_coach_assignments_active ON coach_assignments(user_id, is_active) WHERE is_active = 1;

-- Add index for favorite coach filtering
CREATE INDEX IF NOT EXISTS idx_coach_assignments_favorite ON coach_assignments(user_id, is_favorite) WHERE is_favorite = 1;

-- Drop old indexes that reference columns being removed
DROP INDEX IF EXISTS idx_coaches_favorite;
DROP INDEX IF EXISTS idx_coaches_active;
DROP INDEX IF EXISTS idx_coaches_recent;

-- Drop the preference columns from coaches table
ALTER TABLE coaches DROP COLUMN is_favorite;
ALTER TABLE coaches DROP COLUMN is_active;
ALTER TABLE coaches DROP COLUMN use_count;
ALTER TABLE coaches DROP COLUMN last_used_at;
