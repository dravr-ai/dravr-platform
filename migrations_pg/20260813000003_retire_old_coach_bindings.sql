-- ABOUTME: Drops users.default_coach_id and coach_assignments.is_active (PostgreSQL)
-- ABOUTME: Mirrors the SQLite retirement; both were superseded by tenant_users.selected_coach_id

-- See the matching SQLite migration. Deleting rather than deprecating: a second
-- writable answer to "which coach is this user's?" is how the three-way
-- disagreement arose in the first place, with each surface writing whichever
-- binding it happened to know about.
--
-- coach_assignments remains the roster — assigned_by, is_favorite, use_count,
-- last_used_at all stay. Only the selection flag is retired.

-- The index over the retired flag has to go first: SQLite refuses to drop a
-- column an index still references, and leaving it in PostgreSQL would keep a
-- dead index on a dead column.
DROP INDEX IF EXISTS idx_coach_assignments_active;

ALTER TABLE users DROP COLUMN IF EXISTS default_coach_id;
ALTER TABLE coach_assignments DROP COLUMN IF EXISTS is_active;
