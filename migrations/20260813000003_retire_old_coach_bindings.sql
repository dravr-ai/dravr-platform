-- ABOUTME: Drops users.default_coach_id and coach_assignments.is_active (SQLite)
-- ABOUTME: Both were superseded by tenant_users.selected_coach_id; leaving them invites the drift back

-- Ran after 20260813000002 backfilled every live selection onto the membership
-- row. Deleting rather than deprecating, because a second writable answer to
-- "which coach is this user's?" is exactly how the three-way disagreement
-- started: each surface wrote whichever one it knew about.
--
-- coach_assignments keeps everything that made it a roster — assigned_by,
-- is_favorite, use_count, last_used_at. Only the selection flag goes.

-- The index over the retired flag has to go first: SQLite refuses to drop a
-- column an index still references, and leaving it in PostgreSQL would keep a
-- dead index on a dead column.
DROP INDEX IF EXISTS idx_coach_assignments_active;

ALTER TABLE users DROP COLUMN default_coach_id; -- idempotency-ok: SQLite has no DROP COLUMN IF EXISTS; PG mirror uses IF EXISTS
ALTER TABLE coach_assignments DROP COLUMN is_active; -- idempotency-ok: SQLite has no DROP COLUMN IF EXISTS; PG mirror uses IF EXISTS
