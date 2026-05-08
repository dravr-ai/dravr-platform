-- ABOUTME: 1:N coach-to-athlete roster assignments (SQLite)
-- ABOUTME: Junction table separate from coaching_groups (which binds coach personas, not users)

-- coach_athlete_assignments: a user with manages_roster=true (or admin) is
-- assigned to coach another user within the same tenant. Many-to-many: an
-- athlete can be coached by multiple users, a coach can have many athletes.
-- Soft-deleted via revoked_at so audit history survives the unassign action.
CREATE TABLE IF NOT EXISTS coach_athlete_assignments (
    id TEXT PRIMARY KEY,
    coach_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    athlete_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    assigned_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    assigned_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    revoked_at TEXT,
    revoked_by TEXT REFERENCES users(id) ON DELETE SET NULL
);

-- One active row per (coach, athlete, tenant) — re-assignments after a
-- revoke insert a new row, preserving audit history.
CREATE UNIQUE INDEX IF NOT EXISTS idx_coach_athlete_active
    ON coach_athlete_assignments(coach_user_id, athlete_user_id, tenant_id)
    WHERE revoked_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_coach_athlete_coach
    ON coach_athlete_assignments(coach_user_id, tenant_id, revoked_at);

CREATE INDEX IF NOT EXISTS idx_coach_athlete_athlete
    ON coach_athlete_assignments(athlete_user_id, tenant_id, revoked_at);
