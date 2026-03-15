-- ABOUTME: Consolidated PostgreSQL migration for coaches, store listings, and related tables.
-- ABOUTME: Creates coaches, assignments, relations, versions, authors, store_listings, and user preferences.
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- ============================================================================
-- coaches
-- ============================================================================
-- Final state after all ALTER TABLE additions (migrations 30-39, 203, 218)
-- and column drops (migrations 219, 219002).
CREATE TABLE IF NOT EXISTS coaches (
    id TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,

    -- Coach content
    title TEXT NOT NULL,
    description TEXT,
    system_prompt TEXT NOT NULL,

    -- Categorization
    category TEXT NOT NULL DEFAULT 'custom',
    tags TEXT,

    -- Token tracking
    token_count INTEGER NOT NULL DEFAULT 0,

    -- Metadata
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,

    -- Redesign fields (slug, purpose, instructions, examples, etc.)
    slug TEXT,
    purpose TEXT,
    when_to_use TEXT,
    instructions TEXT,
    example_inputs TEXT,
    example_outputs TEXT,
    success_criteria TEXT,
    prerequisites TEXT,
    source_file TEXT,
    content_hash TEXT,

    -- System coach flags
    is_system BOOLEAN NOT NULL DEFAULT FALSE,
    visibility TEXT NOT NULL DEFAULT 'private',

    -- Sample prompts for coach discovery
    sample_prompts TEXT,

    -- Forking support
    forked_from TEXT REFERENCES coaches(id) ON DELETE SET NULL,

    -- Startup query displayed when coach conversation begins
    startup_query TEXT,

    -- LLM tool iteration limit per conversation turn
    max_tool_iterations INTEGER DEFAULT NULL
);

CREATE INDEX IF NOT EXISTS idx_coaches_user ON coaches(user_id);
CREATE INDEX IF NOT EXISTS idx_coaches_tenant ON coaches(tenant_id);
CREATE INDEX IF NOT EXISTS idx_coaches_category ON coaches(user_id, category);
CREATE UNIQUE INDEX IF NOT EXISTS idx_coaches_slug ON coaches(tenant_id, slug) WHERE slug IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_coaches_source_file ON coaches(source_file) WHERE source_file IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_coaches_system ON coaches(tenant_id, is_system, visibility) WHERE is_system = TRUE;
CREATE INDEX IF NOT EXISTS idx_coaches_forked_from ON coaches(forked_from) WHERE forked_from IS NOT NULL;

-- ============================================================================
-- coach_assignments
-- ============================================================================
-- Final state after migration 32 create + migration 219002 additions.
CREATE TABLE IF NOT EXISTS coach_assignments (
    id TEXT PRIMARY KEY,
    coach_id TEXT NOT NULL REFERENCES coaches(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    assigned_by UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL,
    -- User preference fields moved from coaches table
    is_favorite BOOLEAN NOT NULL DEFAULT FALSE,
    is_active BOOLEAN NOT NULL DEFAULT FALSE,
    use_count INTEGER NOT NULL DEFAULT 0,
    last_used_at TIMESTAMPTZ,
    UNIQUE(coach_id, user_id)
);
CREATE INDEX IF NOT EXISTS idx_coach_assignments_coach ON coach_assignments(coach_id);
CREATE INDEX IF NOT EXISTS idx_coach_assignments_user ON coach_assignments(user_id);
CREATE INDEX IF NOT EXISTS idx_coach_assignments_active ON coach_assignments(user_id, is_active) WHERE is_active = TRUE;
CREATE INDEX IF NOT EXISTS idx_coach_assignments_favorite ON coach_assignments(user_id, is_favorite) WHERE is_favorite = TRUE;

-- ============================================================================
-- coach_relations
-- ============================================================================
CREATE TABLE IF NOT EXISTS coach_relations (
    id TEXT PRIMARY KEY,
    coach_id TEXT NOT NULL REFERENCES coaches(id) ON DELETE CASCADE,
    related_coach_id TEXT NOT NULL REFERENCES coaches(id) ON DELETE CASCADE,
    relation_type TEXT NOT NULL CHECK (relation_type IN ('related', 'alternative', 'prerequisite', 'sequel')),
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE(coach_id, related_coach_id, relation_type)
);
CREATE INDEX IF NOT EXISTS idx_coach_relations_coach ON coach_relations(coach_id);
CREATE INDEX IF NOT EXISTS idx_coach_relations_related ON coach_relations(related_coach_id);

-- ============================================================================
-- coach_versions
-- ============================================================================
CREATE TABLE IF NOT EXISTS coach_versions (
    id TEXT PRIMARY KEY,
    coach_id TEXT NOT NULL REFERENCES coaches(id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    content_snapshot TEXT NOT NULL,
    change_summary TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    created_by UUID REFERENCES users(id) ON DELETE SET NULL,
    UNIQUE(coach_id, version)
);
CREATE INDEX IF NOT EXISTS idx_coach_versions_coach ON coach_versions(coach_id, version DESC);

-- ============================================================================
-- coach_authors
-- ============================================================================
CREATE TABLE IF NOT EXISTS coach_authors (
    id TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    bio TEXT,
    avatar_url TEXT,
    website_url TEXT,
    is_verified BOOLEAN NOT NULL DEFAULT FALSE,
    verified_at TIMESTAMPTZ,
    verified_by UUID REFERENCES users(id) ON DELETE SET NULL,
    published_coach_count INTEGER NOT NULL DEFAULT 0,
    total_install_count INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE(user_id, tenant_id)
    -- tenant_id is TEXT (application-level join to tenants.id UUID — no FK constraint)
);
CREATE INDEX IF NOT EXISTS idx_coach_authors_user ON coach_authors(user_id, tenant_id);
CREATE INDEX IF NOT EXISTS idx_coach_authors_verified ON coach_authors(is_verified, total_install_count DESC) WHERE is_verified = TRUE;
CREATE INDEX IF NOT EXISTS idx_coach_authors_popular ON coach_authors(total_install_count DESC) WHERE published_coach_count > 0;

-- ============================================================================
-- store_listings
-- ============================================================================
CREATE TABLE IF NOT EXISTS store_listings (
    id TEXT PRIMARY KEY,
    coach_id TEXT NOT NULL UNIQUE REFERENCES coaches(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    publish_status TEXT NOT NULL DEFAULT 'draft',
    published_at TIMESTAMPTZ,
    review_submitted_at TIMESTAMPTZ,
    review_decision_at TIMESTAMPTZ,
    review_decision_by TEXT,
    rejection_reason TEXT,
    install_count INTEGER NOT NULL DEFAULT 0,
    icon_url TEXT,
    author_id TEXT REFERENCES coach_authors(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_store_listings_coach_id ON store_listings(coach_id);
CREATE INDEX IF NOT EXISTS idx_store_listings_status ON store_listings(publish_status);
CREATE INDEX IF NOT EXISTS idx_store_listings_tenant ON store_listings(tenant_id);
CREATE INDEX IF NOT EXISTS idx_store_listings_published_at ON store_listings(published_at);

-- ============================================================================
-- user_coach_preferences
-- ============================================================================
CREATE TABLE IF NOT EXISTS user_coach_preferences (
    id TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    coach_id TEXT NOT NULL REFERENCES coaches(id) ON DELETE CASCADE,
    is_hidden BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE(user_id, coach_id)
);
CREATE INDEX IF NOT EXISTS idx_user_coach_prefs_user ON user_coach_preferences(user_id);
