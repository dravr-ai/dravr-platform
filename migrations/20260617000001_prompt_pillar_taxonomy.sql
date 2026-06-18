-- ABOUTME: Rebuilds prompt_suggestions to the canonical six-pillar taxonomy CHECK
-- ABOUTME: and backfills legacy activity/nutrition/recovery pillars to the new slugs.

-- sqlx runs each migration in a transaction; `PRAGMA foreign_keys` is a no-op
-- inside a transaction, so defer FK checks to COMMIT instead. SQLite cannot alter
-- a CHECK constraint in place, so the table is rebuilt with the widened CHECK.
PRAGMA defer_foreign_keys = ON;

DROP TABLE IF EXISTS prompt_suggestions_new;
CREATE TABLE IF NOT EXISTS prompt_suggestions_new (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    category_key TEXT NOT NULL,
    category_title TEXT NOT NULL,
    category_icon TEXT NOT NULL,
    pillar TEXT NOT NULL CHECK (pillar IN (
        'training_and_movement', 'fuelling', 'sleep_and_recovery',
        'mental_resilience', 'community_and_connection', 'recovery_optimisation'
    )),
    prompts TEXT NOT NULL,  -- JSON array of prompt strings
    display_order INTEGER NOT NULL DEFAULT 0,
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(tenant_id, category_key)
);

-- Map the legacy 3-pillar values onto the canonical six. The ELSE is a lenient
-- catch-all so any unexpected legacy value lands on training_and_movement
-- rather than violating the new CHECK and aborting the migration.
INSERT INTO prompt_suggestions_new (id, tenant_id, category_key, category_title,
    category_icon, pillar, prompts, display_order, is_active, created_at, updated_at)
  SELECT id, tenant_id, category_key, category_title, category_icon,
    CASE pillar
        WHEN 'activity' THEN 'training_and_movement'
        WHEN 'nutrition' THEN 'fuelling'
        WHEN 'recovery' THEN 'sleep_and_recovery'
        ELSE 'training_and_movement'
    END,
    prompts, display_order, is_active, created_at, updated_at
  FROM prompt_suggestions;

DROP TABLE IF EXISTS prompt_suggestions;
ALTER TABLE prompt_suggestions_new RENAME TO prompt_suggestions;

-- Recreate the indexes against the rebuilt table.
CREATE INDEX IF NOT EXISTS idx_prompt_suggestions_tenant ON prompt_suggestions(tenant_id);
CREATE INDEX IF NOT EXISTS idx_prompt_suggestions_active ON prompt_suggestions(tenant_id, is_active);
CREATE INDEX IF NOT EXISTS idx_prompt_suggestions_order ON prompt_suggestions(tenant_id, display_order);
