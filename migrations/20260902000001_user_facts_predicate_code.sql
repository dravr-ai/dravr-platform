-- ABOUTME: Replaces user_facts.subject + predicate (free English text) with a closed predicate_code
-- ABOUTME: Backfills the seven server-authored phrases to codes; every other row keeps its words under 'states'

-- The extraction LLM wrote predicates in English and every renderer glued
-- them to the object, so a French athlete read half-English sentences. The
-- predicate is now a code from a closed vocabulary; the object is the
-- athlete's own words; the sentence is rendered per locale from the string
-- catalogue. SQLite cannot drop columns inside a CHECKed table cleanly, so the
-- table is rebuilt (same shape as 20260617000002).
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
        'training_for', 'working_toward', 'target_race', 'prefer', 'avoid', 'primarily_train',
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
    embedding BLOB,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- The seven phrases the server itself authored map to their codes and keep
-- their object. Every other row was free text from the extractor: it becomes
-- 'states' with the old sentence folded into the object (subject first when
-- it named someone other than the athlete), so nothing is lost and nothing
-- pretends to be structured.
INSERT INTO user_facts_new (
    id, tenant_id, user_id, coach_id, scope, kind, pillar, predicate_code,
    object, confidence, source, valid_until, source_msg_id, embedding,
    created_at, updated_at
)
  SELECT id, tenant_id, user_id, coach_id, scope, kind, pillar,
    CASE predicate
        WHEN 'train because' THEN 'train_because'
        WHEN 'primarily train' THEN 'primarily_train'
        WHEN 'are working toward' THEN 'working_toward'
        WHEN 'want' THEN 'working_toward'
        WHEN 'answered yes (PAR-Q)' THEN 'parq_yes'
        WHEN 'target race' THEN 'target_race'
        WHEN 'flagged' THEN 'flagged'
        ELSE 'states'
    END,
    CASE
        WHEN predicate IN ('train because', 'primarily train', 'are working toward', 'want', 'answered yes (PAR-Q)', 'target race', 'flagged') THEN object
        WHEN lower(trim(subject)) = 'you' THEN trim(predicate || ' ' || object)
        ELSE trim(subject || ' ' || predicate || ' ' || object)
    END,
    confidence, source, valid_until, source_msg_id, embedding, created_at, updated_at
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
