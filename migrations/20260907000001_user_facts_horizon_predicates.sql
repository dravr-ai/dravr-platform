-- ABOUTME: Adds the two season-horizon goal codes to user_facts.predicate_code's closed vocabulary
-- ABOUTME: The SQLite half: a CHECK cannot be altered in place, so the table is rebuilt (same shape as 20260902000001)

-- The /season walk asks what "good" looks like this season and, separately,
-- in a few years, and files each under its own code — aim_this_season and
-- aim_long_term — because about_you supersedes every 'working_toward' fact on
-- each write, and two horizons under one code would clobber each other.
--
-- The table shape is 20260902000001's without the embedding column that
-- 20260903000002 dropped. Every row carries over unchanged.

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
    predicate_code TEXT NOT NULL CHECK (predicate_code IN (
        'training_for', 'working_toward', 'target_race', 'aim_this_season', 'aim_long_term',
        'prefer', 'avoid', 'primarily_train',
        'have_baseline', 'have', 'recovering_from', 'can_train_on', 'cannot_train_on',
        'need_session_on', 'unavailable', 'own', 'train_on', 'train_because', 'parq_yes',
        'flagged', 'states'
    )),
    object TEXT NOT NULL,
    confidence REAL NOT NULL DEFAULT 0.0,
    source TEXT NOT NULL DEFAULT 'conversation' CHECK (source IN (
        'onboarding', 'conversation', 'device', 'coach'
    )),
    valid_until TEXT,
    source_msg_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

INSERT INTO user_facts_new (
    id, tenant_id, user_id, coach_id, scope, kind, pillar, predicate_code,
    object, confidence, source, valid_until, source_msg_id, created_at, updated_at
)
  SELECT id, tenant_id, user_id, coach_id, scope, kind, pillar, predicate_code,
    object, confidence, source, valid_until, source_msg_id, created_at, updated_at
  FROM user_facts;

DROP TABLE IF EXISTS user_facts;
ALTER TABLE user_facts_new RENAME TO user_facts;

CREATE INDEX IF NOT EXISTS idx_user_facts_tenant_user
    ON user_facts(tenant_id, user_id);
CREATE INDEX IF NOT EXISTS idx_user_facts_tenant_user_coach
    ON user_facts(tenant_id, user_id, coach_id)
    WHERE coach_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_user_facts_kind
    ON user_facts(tenant_id, user_id, kind);
CREATE INDEX IF NOT EXISTS idx_user_facts_updated_at
    ON user_facts(updated_at DESC);
