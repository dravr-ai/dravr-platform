-- ABOUTME: Add the 'athlete_data' claim category and its matching verdict layer (PostgreSQL)
-- ABOUTME: Postgres supports ALTER ... DROP/ADD CONSTRAINT directly (no table rebuild)

-- See the SQLite mirror for why this category exists: the six existing ones are
-- sports-science propositions checked against a literature corpus, and a claim
-- about the athlete's own records is a database question with nowhere to land.

ALTER TABLE claim_verdicts DROP CONSTRAINT IF EXISTS claim_verdicts_category_check;
ALTER TABLE claim_verdicts ADD CONSTRAINT claim_verdicts_category_check
    CHECK (category IN (
        'physiological',
        'training_prescription',
        'nutrition',
        'recovery',
        'supplement',
        'injury_rehab',
        'athlete_data'
    ));

ALTER TABLE claim_verdicts DROP CONSTRAINT IF EXISTS claim_verdicts_layer_fired_check;
ALTER TABLE claim_verdicts ADD CONSTRAINT claim_verdicts_layer_fired_check
    CHECK (layer_fired IN (
        'rhetoric',
        'deterministic',
        'personalized',
        'athlete_data',
        'evidence',
        'consistency',
        'judge'
    ));
