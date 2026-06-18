-- ABOUTME: Migrates prompt_suggestions.pillar to the canonical six-pillar taxonomy
-- ABOUTME: by backfilling legacy values then swapping the CHECK constraint in place.

-- Backfill BEFORE swapping the constraint so no row violates the new CHECK.
-- The ELSE is a lenient catch-all so any unexpected legacy value lands on
-- training_and_movement rather than blocking the constraint swap.
UPDATE prompt_suggestions
   SET pillar = CASE pillar
        WHEN 'activity' THEN 'training_and_movement'
        WHEN 'nutrition' THEN 'fuelling'
        WHEN 'recovery' THEN 'sleep_and_recovery'
        ELSE 'training_and_movement'
   END;

-- Inline column CHECK gets the default name prompt_suggestions_pillar_check.
ALTER TABLE prompt_suggestions DROP CONSTRAINT IF EXISTS prompt_suggestions_pillar_check;
ALTER TABLE prompt_suggestions ADD CONSTRAINT prompt_suggestions_pillar_check CHECK (pillar IN (
    'training_and_movement', 'fuelling', 'sleep_and_recovery',
    'mental_resilience', 'community_and_connection', 'recovery_optimisation'
));
