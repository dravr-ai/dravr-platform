-- ABOUTME: Schema for coaching groups, membership, and invite system
-- ABOUTME: Enables multi-person AI coaching with privacy-gated peer sharing

-- coaching_groups: binds a coach persona to multiple athletes
CREATE TABLE IF NOT EXISTS coaching_groups (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    coach_id TEXT NOT NULL REFERENCES coaches(id),
    owner_id UUID NOT NULL REFERENCES users(id),
    peer_data_sharing BOOLEAN NOT NULL DEFAULT FALSE,
    max_members INTEGER NOT NULL DEFAULT 20,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_coaching_groups_tenant ON coaching_groups(tenant_id);
CREATE INDEX IF NOT EXISTS idx_coaching_groups_coach ON coaching_groups(coach_id);
CREATE INDEX IF NOT EXISTS idx_coaching_groups_owner ON coaching_groups(owner_id);

-- coaching_group_members: membership with role and consent tracking
CREATE TABLE IF NOT EXISTS coaching_group_members (
    id TEXT PRIMARY KEY,
    group_id TEXT NOT NULL REFERENCES coaching_groups(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id),
    tenant_id TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('member', 'admin', 'owner')),
    peer_sharing_consent BOOLEAN NOT NULL DEFAULT FALSE,
    consent_given_at TIMESTAMPTZ NOT NULL,
    joined_at TIMESTAMPTZ NOT NULL,
    left_at TIMESTAMPTZ,
    UNIQUE(group_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_group_members_group ON coaching_group_members(group_id);
CREATE INDEX IF NOT EXISTS idx_group_members_user ON coaching_group_members(user_id);
CREATE INDEX IF NOT EXISTS idx_group_members_tenant ON coaching_group_members(tenant_id);

-- group_invites: invite codes with optional expiration and use limits
CREATE TABLE IF NOT EXISTS group_invites (
    id TEXT PRIMARY KEY,
    group_id TEXT NOT NULL REFERENCES coaching_groups(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    code TEXT NOT NULL UNIQUE,
    created_by UUID NOT NULL REFERENCES users(id),
    expires_at TIMESTAMPTZ,
    max_uses INTEGER,
    use_count INTEGER NOT NULL DEFAULT 0,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_group_invites_code ON group_invites(code);
CREATE INDEX IF NOT EXISTS idx_group_invites_group ON group_invites(group_id);

-- Link conversations to group context for disambiguation
ALTER TABLE chat_conversations ADD COLUMN group_id TEXT REFERENCES coaching_groups(id);
