-- ABOUTME: Adds pillar / source / valid_until to user_facts and widens the kind
-- ABOUTME: CHECK to include north_star + medical, for per-user pillar context.

-- sqlx runs each migration in a transaction; `PRAGMA foreign_keys` is a no-op
-- inside a transaction, so defer FK checks to COMMIT. SQLite cannot widen the
-- existing `kind` CHECK in place, so the table is rebuilt with all the new
-- columns and the widened CHECK in one pass.
PRAGMA defer_foreign_keys = ON;

DROP TABLE IF EXISTS user_facts_new;
CREATE TABLE IF NOT EXISTS user_facts_new (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    coach_id TEXT REFERENCES coaches(id) ON DELETE SET NULL,
    scope TEXT NOT NULL CHECK (scope IN ('conversation', 'user', 'tenant')),
    kind TEXT NOT NULL CHECK (kind IN (
        'preference', 'physiology', 'injury', 'goal',
        'schedule', 'equipment', 'north_star', 'medical', 'other'
    )),
    pillar TEXT CHECK (pillar IS NULL OR pillar IN (
        'training_and_movement', 'fuelling', 'sleep_and_recovery',
        'mental_resilience', 'community_and_connection', 'recovery_optimisation'
    )),
    subject TEXT NOT NULL,
    predicate TEXT NOT NULL,
    object TEXT NOT NULL,
    confidence REAL NOT NULL DEFAULT 0.0,
    source TEXT NOT NULL DEFAULT 'conversation' CHECK (source IN (
        'onboarding', 'conversation', 'device', 'coach'
    )),
    valid_until TEXT,
    source_msg_id TEXT,
    embedding BLOB,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Existing rows predate pillar tagging: pillar -> NULL, valid_until -> NULL,
-- source defaults to 'conversation' (they were extracted from chat turns).
INSERT INTO user_facts_new (
    id, tenant_id, user_id, coach_id, scope, kind, subject, predicate,
    object, confidence, source_msg_id, embedding, created_at, updated_at
)
  SELECT id, tenant_id, user_id, coach_id, scope, kind, subject, predicate,
    object, confidence, source_msg_id, embedding, created_at, updated_at
  FROM user_facts;

DROP TABLE IF EXISTS user_facts;
ALTER TABLE user_facts_new RENAME TO user_facts;

-- Recreate the indexes against the rebuilt table.
CREATE INDEX IF NOT EXISTS idx_user_facts_tenant_user
    ON user_facts(tenant_id, user_id);
CREATE INDEX IF NOT EXISTS idx_user_facts_tenant_user_coach
    ON user_facts(tenant_id, user_id, coach_id)
    WHERE coach_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_user_facts_kind
    ON user_facts(tenant_id, user_id, kind);
CREATE INDEX IF NOT EXISTS idx_user_facts_updated_at
    ON user_facts(updated_at DESC);
