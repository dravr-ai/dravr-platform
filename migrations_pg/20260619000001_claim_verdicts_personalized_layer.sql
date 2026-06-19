-- ABOUTME: Add 'personalized' (Layer 2.5) to the claim_verdicts.layer_fired CHECK constraint
-- ABOUTME: Postgres supports ALTER ... DROP/ADD CONSTRAINT directly (no table rebuild)

ALTER TABLE claim_verdicts DROP CONSTRAINT IF EXISTS claim_verdicts_layer_fired_check;
ALTER TABLE claim_verdicts ADD CONSTRAINT claim_verdicts_layer_fired_check
    CHECK (layer_fired IN (
        'rhetoric',
        'deterministic',
        'personalized',
        'evidence',
        'consistency',
        'judge'
    ));
