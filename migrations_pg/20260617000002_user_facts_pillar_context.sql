-- ABOUTME: Adds pillar / source / valid_until to user_facts and widens the kind
-- ABOUTME: CHECK to include north_star + medical, for per-user pillar context.

-- Postgres supports in-place column adds and constraint swaps, so no rebuild.
ALTER TABLE user_facts ADD COLUMN IF NOT EXISTS pillar TEXT;
ALTER TABLE user_facts DROP CONSTRAINT IF EXISTS user_facts_pillar_check;
ALTER TABLE user_facts ADD CONSTRAINT user_facts_pillar_check CHECK (pillar IS NULL OR pillar IN (
    'training_and_movement', 'fuelling', 'sleep_and_recovery',
    'mental_resilience', 'community_and_connection', 'recovery_optimisation'
));

ALTER TABLE user_facts ADD COLUMN IF NOT EXISTS valid_until TIMESTAMPTZ;

ALTER TABLE user_facts ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'conversation';
ALTER TABLE user_facts DROP CONSTRAINT IF EXISTS user_facts_source_check;
ALTER TABLE user_facts ADD CONSTRAINT user_facts_source_check CHECK (source IN (
    'onboarding', 'conversation', 'device', 'coach'
));

-- Widen the kind CHECK to admit north_star + medical.
ALTER TABLE user_facts DROP CONSTRAINT IF EXISTS user_facts_kind_check;
ALTER TABLE user_facts ADD CONSTRAINT user_facts_kind_check CHECK (kind IN (
    'preference', 'physiology', 'injury', 'goal',
    'schedule', 'equipment', 'north_star', 'medical', 'other'
));
