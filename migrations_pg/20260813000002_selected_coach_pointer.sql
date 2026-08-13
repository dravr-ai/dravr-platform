-- ABOUTME: tenant_users.selected_coach_id — the one pointer to a user's current coach (PostgreSQL)
-- ABOUTME: Mirrors the SQLite migration, including the backfill from both retired bindings

-- See the matching SQLite migration for the full rationale. Summary: three
-- bindings answered "which coach is this user's?" and disagreed, so a user who
-- finished the web wizard still read as having no active coach and got
-- re-onboarded on their first messaging turn.
--
-- The shape is the standard one for "which of these is current": a foreign key on
-- the entity that owns the choice, not a boolean on each collection row.
-- Maintaining "at most one" through coach_assignments.is_active took two
-- non-transactional UPDATEs and could leave zero or two; a single column cannot.
--
-- It sits on tenant_users because coaches are tenant-scoped and the retired
-- users.default_coach_id carried no tenant, so a user in two tenants had one
-- global coach that silently failed to resolve in the other.

ALTER TABLE tenant_users
    ADD COLUMN IF NOT EXISTS selected_coach_id TEXT REFERENCES coaches(id) ON DELETE SET NULL;

-- Carry existing selections over: users.default_coach_id first (the binding that
-- actually drove replies), then any active assignment, so nobody loses the coach
-- they were already talking to.
UPDATE tenant_users tu
   SET selected_coach_id = u.default_coach_id
  FROM users u
 WHERE u.id = tu.user_id
   AND tu.selected_coach_id IS NULL
   AND u.default_coach_id IS NOT NULL;

UPDATE tenant_users tu
   SET selected_coach_id = ca.coach_id
  FROM coach_assignments ca
 WHERE ca.user_id::text = tu.user_id::text
   -- `is_active` is BOOLEAN here (20260311000005), not the 0/1 integer SQLite
   -- stores. `= 1` is a type error Postgres raises at plan time, so it failed
   -- the whole migration and every test that builds a database from it.
   AND ca.is_active
   AND tu.selected_coach_id IS NULL;

CREATE INDEX IF NOT EXISTS idx_tenant_users_selected_coach ON tenant_users(selected_coach_id);
