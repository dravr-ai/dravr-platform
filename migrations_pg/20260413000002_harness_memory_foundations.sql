-- ABOUTME: Tier 0 memory foundations (PostgreSQL) — compaction, facts, notes, followups, sessions
-- ABOUTME: pgvector extension optional; embeddings are BYTEA for SQLite parity, JSONB for metadata

-- pgvector is the idiomatic Postgres choice but isn't available in every
-- managed Postgres deployment. To keep parity with the SQLite path, we
-- store embeddings as BYTEA (little-endian f32 sequence) and do vector math
-- in Rust. A follow-up migration can promote these columns to `vector(N)`
-- once pgvector is provisioned in every environment.

CREATE TABLE IF NOT EXISTS compaction_blocks (
    id VARCHAR(255) PRIMARY KEY,
    tenant_id VARCHAR(255) NOT NULL,
    conversation_id VARCHAR(255) NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
    summary TEXT NOT NULL,
    summary_tokens INTEGER NOT NULL DEFAULT 0,
    original_tokens INTEGER NOT NULL DEFAULT 0,
    first_message_id VARCHAR(255) NOT NULL,
    last_message_id VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_compaction_blocks_conversation
    ON compaction_blocks(conversation_id);
CREATE INDEX IF NOT EXISTS idx_compaction_blocks_tenant_created
    ON compaction_blocks(tenant_id, created_at DESC);

CREATE TABLE IF NOT EXISTS user_facts (
    id VARCHAR(255) PRIMARY KEY,
    tenant_id VARCHAR(255) NOT NULL,
    user_id VARCHAR(255) NOT NULL,
    coach_id TEXT REFERENCES coaches(id) ON DELETE SET NULL,
    scope VARCHAR(20) NOT NULL CHECK (scope IN ('conversation', 'user', 'tenant')),
    kind VARCHAR(20) NOT NULL CHECK (kind IN (
        'preference', 'physiology', 'injury', 'goal',
        'schedule', 'equipment', 'other'
    )),
    subject TEXT NOT NULL,
    predicate TEXT NOT NULL,
    object TEXT NOT NULL,
    confidence REAL NOT NULL DEFAULT 0.0,
    source_msg_id VARCHAR(255),
    embedding BYTEA,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_user_facts_tenant_user
    ON user_facts(tenant_id, user_id);
CREATE INDEX IF NOT EXISTS idx_user_facts_tenant_user_coach
    ON user_facts(tenant_id, user_id, coach_id)
    WHERE coach_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_user_facts_kind
    ON user_facts(tenant_id, user_id, kind);
CREATE INDEX IF NOT EXISTS idx_user_facts_updated_at
    ON user_facts(updated_at DESC);

CREATE TABLE IF NOT EXISTS coach_notes (
    id VARCHAR(255) PRIMARY KEY,
    tenant_id VARCHAR(255) NOT NULL,
    user_id VARCHAR(255) NOT NULL,
    coach_id TEXT NOT NULL REFERENCES coaches(id) ON DELETE CASCADE,
    conversation_id VARCHAR(255) REFERENCES chat_conversations(id) ON DELETE SET NULL,
    scope VARCHAR(20) NOT NULL CHECK (scope IN ('conversation', 'user', 'tenant')),
    content TEXT NOT NULL,
    embedding BYTEA,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_coach_notes_tenant_user_coach
    ON coach_notes(tenant_id, user_id, coach_id);
CREATE INDEX IF NOT EXISTS idx_coach_notes_created_at
    ON coach_notes(created_at DESC);

CREATE TABLE IF NOT EXISTS coach_followups (
    id VARCHAR(255) PRIMARY KEY,
    tenant_id VARCHAR(255) NOT NULL,
    user_id VARCHAR(255) NOT NULL,
    coach_id TEXT NOT NULL REFERENCES coaches(id) ON DELETE CASCADE,
    conversation_id VARCHAR(255) REFERENCES chat_conversations(id) ON DELETE SET NULL,
    content TEXT NOT NULL,
    due_at TIMESTAMPTZ,
    status VARCHAR(20) NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'delivered', 'cancelled')),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    delivered_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS idx_coach_followups_pending_due
    ON coach_followups(tenant_id, user_id, coach_id, due_at)
    WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS idx_coach_followups_tenant_user_coach
    ON coach_followups(tenant_id, user_id, coach_id);

CREATE TABLE IF NOT EXISTS coach_sessions (
    id VARCHAR(255) PRIMARY KEY,
    tenant_id VARCHAR(255) NOT NULL,
    user_id VARCHAR(255) NOT NULL,
    coach_id TEXT NOT NULL REFERENCES coaches(id) ON DELETE CASCADE,
    status VARCHAR(20) NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived')),
    opened_at TIMESTAMPTZ NOT NULL,
    last_turn_at TIMESTAMPTZ,
    archived_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_coach_sessions_tenant_user_coach
    ON coach_sessions(tenant_id, user_id, coach_id);
CREATE INDEX IF NOT EXISTS idx_coach_sessions_active_last_turn
    ON coach_sessions(tenant_id, user_id, last_turn_at DESC)
    WHERE status = 'active';

ALTER TABLE chat_conversations
    ADD COLUMN IF NOT EXISTS session_id VARCHAR(255) REFERENCES coach_sessions(id) ON DELETE SET NULL;
CREATE INDEX IF NOT EXISTS idx_chat_conversations_session
    ON chat_conversations(session_id)
    WHERE session_id IS NOT NULL;
