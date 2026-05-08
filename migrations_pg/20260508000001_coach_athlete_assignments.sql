-- ABOUTME: 1:N coach-to-athlete roster assignments (PostgreSQL)
-- ABOUTME: Junction table separate from coaching_groups (which binds coach personas, not users)

CREATE TABLE IF NOT EXISTS coach_athlete_assignments (
    id UUID PRIMARY KEY,
    coach_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    athlete_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    assigned_by UUID REFERENCES users(id) ON DELETE SET NULL,
    assigned_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ,
    revoked_by UUID REFERENCES users(id) ON DELETE SET NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_coach_athlete_active
    ON coach_athlete_assignments(coach_user_id, athlete_user_id, tenant_id)
    WHERE revoked_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_coach_athlete_coach
    ON coach_athlete_assignments(coach_user_id, tenant_id, revoked_at);

CREATE INDEX IF NOT EXISTS idx_coach_athlete_athlete
    ON coach_athlete_assignments(athlete_user_id, tenant_id, revoked_at);
