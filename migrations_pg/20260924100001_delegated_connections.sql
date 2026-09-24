-- ABOUTME: A member's provider read through their group coach's session, proposed by the coach and confirmed by the member (PostgreSQL version)
-- ABOUTME: Rows are terminal once revoked and stay for audit; the partial unique indexes cover live rows only

-- One row per link. The coach proposes (a TrainingPeaks athlete on their
-- roster <-> a live member of a group they coach); the member confirms, and
-- that confirmation is their consent to the read. provider_athlete_name is the
-- name the coach's TrainingPeaks roster shows: untrusted third-party text,
-- rendered by the apps and never put into a prompt.
CREATE TABLE IF NOT EXISTS delegated_connections (
    id UUID PRIMARY KEY,
    provider TEXT NOT NULL,
    group_id UUID NOT NULL REFERENCES coaching_groups(id) ON DELETE CASCADE,
    coach_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    coach_tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    member_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    member_tenant_id UUID REFERENCES tenants(id) ON DELETE CASCADE,
    provider_athlete_id TEXT NOT NULL,
    provider_athlete_name TEXT,
    status TEXT NOT NULL CHECK (status IN ('proposed', 'confirmed', 'revoked')),
    proposed_at TIMESTAMPTZ NOT NULL,
    confirmed_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    revoked_by UUID REFERENCES users(id) ON DELETE SET NULL,
    revoke_reason TEXT CHECK (revoke_reason IS NULL OR revoke_reason IN (
        'declined', 'withdrawn', 'revoked_by_member', 'revoked_by_coach', 'member_left',
        'member_removed', 'coach_detached', 'group_archived', 'coach_disconnected',
        'not_on_roster', 'superseded')),
    CHECK (status <> 'confirmed' OR (confirmed_at IS NOT NULL AND member_tenant_id IS NOT NULL)),
    CHECK (status <> 'revoked' OR revoked_at IS NOT NULL)
);

-- A member reads a provider through at most one confirmed link.
CREATE UNIQUE INDEX IF NOT EXISTS uq_delegated_connections_member_confirmed
    ON delegated_connections(member_user_id, provider) WHERE status = 'confirmed';

-- One live link per member per group per provider.
CREATE UNIQUE INDEX IF NOT EXISTS uq_delegated_connections_group_member_live
    ON delegated_connections(group_id, member_user_id, provider)
    WHERE status IN ('proposed', 'confirmed');

-- A coach links one roster athlete to one member at a time.
CREATE UNIQUE INDEX IF NOT EXISTS uq_delegated_connections_coach_athlete_live
    ON delegated_connections(coach_user_id, provider, provider_athlete_id)
    WHERE status IN ('proposed', 'confirmed');

CREATE INDEX IF NOT EXISTS idx_delegated_connections_group
    ON delegated_connections(group_id, status);
CREATE INDEX IF NOT EXISTS idx_delegated_connections_coach
    ON delegated_connections(coach_user_id, coach_tenant_id, provider, status);
CREATE INDEX IF NOT EXISTS idx_delegated_connections_member
    ON delegated_connections(member_user_id, member_tenant_id, provider, status);
