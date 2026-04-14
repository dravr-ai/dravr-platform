-- ABOUTME: Tier 0 memory foundations — dormant schemas for compaction, facts, notes, followups, sessions
-- ABOUTME: Tables land without services; Tier 1+ wires them up per the coaching harness plan

-- Compaction blocks replace earlier conversation turns in long sessions.
-- Stored as first-class rows so the UI can render "earlier summary" callouts
-- and admins can debug why a given fact was or wasn't preserved.
CREATE TABLE IF NOT EXISTS compaction_blocks (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
    summary TEXT NOT NULL,
    summary_tokens INTEGER NOT NULL DEFAULT 0,
    original_tokens INTEGER NOT NULL DEFAULT 0,
    first_message_id TEXT NOT NULL,
    last_message_id TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_compaction_blocks_conversation
    ON compaction_blocks(conversation_id);
CREATE INDEX IF NOT EXISTS idx_compaction_blocks_tenant_created
    ON compaction_blocks(tenant_id, created_at DESC);

-- User facts are structured claims the harness has extracted about a user.
-- `embedding` stores the vector as a BLOB (little-endian f32 triples) so
-- SQLite can ship the column without loading a custom extension. Vector
-- math is done in Rust when recalling.
CREATE TABLE IF NOT EXISTS user_facts (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    coach_id TEXT REFERENCES coaches(id) ON DELETE SET NULL,
    scope TEXT NOT NULL CHECK (scope IN ('conversation', 'user', 'tenant')),
    kind TEXT NOT NULL CHECK (kind IN (
        'preference', 'physiology', 'injury', 'goal',
        'schedule', 'equipment', 'other'
    )),
    subject TEXT NOT NULL,
    predicate TEXT NOT NULL,
    object TEXT NOT NULL,
    confidence REAL NOT NULL DEFAULT 0.0,
    source_msg_id TEXT,
    embedding BLOB,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
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

-- Coach notes are what the coach persona intentionally wrote via a tool call
-- (distinct from user_facts which the background extractor infers).
CREATE TABLE IF NOT EXISTS coach_notes (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    coach_id TEXT NOT NULL REFERENCES coaches(id) ON DELETE CASCADE,
    conversation_id TEXT REFERENCES chat_conversations(id) ON DELETE SET NULL,
    scope TEXT NOT NULL CHECK (scope IN ('conversation', 'user', 'tenant')),
    content TEXT NOT NULL,
    embedding BLOB,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_coach_notes_tenant_user_coach
    ON coach_notes(tenant_id, user_id, coach_id);
CREATE INDEX IF NOT EXISTS idx_coach_notes_created_at
    ON coach_notes(created_at DESC);

-- Coach followups are promised future check-ins. Pending rows get injected
-- into the next session's system prompt.
CREATE TABLE IF NOT EXISTS coach_followups (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    coach_id TEXT NOT NULL REFERENCES coaches(id) ON DELETE CASCADE,
    conversation_id TEXT REFERENCES chat_conversations(id) ON DELETE SET NULL,
    content TEXT NOT NULL,
    due_at TEXT,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'delivered', 'cancelled')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    delivered_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_coach_followups_pending_due
    ON coach_followups(tenant_id, user_id, coach_id, due_at)
    WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS idx_coach_followups_tenant_user_coach
    ON coach_followups(tenant_id, user_id, coach_id);

-- Coach sessions are long-lived (user, coach) containers above chat_conversations.
-- Tier 4 wires this up; Tier 0 lands the schema dormant.
CREATE TABLE IF NOT EXISTS coach_sessions (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    coach_id TEXT NOT NULL REFERENCES coaches(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived')),
    opened_at TEXT NOT NULL,
    last_turn_at TEXT,
    archived_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_coach_sessions_tenant_user_coach
    ON coach_sessions(tenant_id, user_id, coach_id);
CREATE INDEX IF NOT EXISTS idx_coach_sessions_active_last_turn
    ON coach_sessions(tenant_id, user_id, last_turn_at DESC)
    WHERE status = 'active';

-- Link chat_conversations to a session. Nullable so pre-Tier-4 rows remain
-- valid; the Tier 4 branch will backfill one session per (user, coach) pair
-- from existing conversations.
ALTER TABLE chat_conversations
    ADD COLUMN session_id TEXT REFERENCES coach_sessions(id) ON DELETE SET NULL;
CREATE INDEX IF NOT EXISTS idx_chat_conversations_session
    ON chat_conversations(session_id)
    WHERE session_id IS NOT NULL;
