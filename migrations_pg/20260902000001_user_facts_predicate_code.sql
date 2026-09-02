-- ABOUTME: Replaces user_facts.subject + predicate (free English text) with a closed predicate_code
-- ABOUTME: Backfills the seven server-authored phrases to codes; every other row keeps its words under 'states'

-- Postgres adds the column in place, backfills, then drops the free-text
-- columns; every statement is re-runnable.
ALTER TABLE user_facts ADD COLUMN IF NOT EXISTS predicate_code TEXT;

UPDATE user_facts
   SET predicate_code = CASE predicate
        WHEN 'train because' THEN 'train_because'
        WHEN 'primarily train' THEN 'primarily_train'
        WHEN 'are working toward' THEN 'working_toward'
        WHEN 'want' THEN 'working_toward'
        WHEN 'answered yes (PAR-Q)' THEN 'parq_yes'
        WHEN 'target race' THEN 'target_race'
        WHEN 'flagged' THEN 'flagged'
        ELSE 'states'
    END,
       object = CASE
           WHEN predicate IN ('train because', 'primarily train', 'are working toward', 'want', 'answered yes (PAR-Q)', 'target race', 'flagged') THEN object
           WHEN lower(trim(subject)) = 'you' THEN trim(predicate || ' ' || object)
           ELSE trim(subject || ' ' || predicate || ' ' || object)
       END
 WHERE predicate_code IS NULL
   AND EXISTS (
       SELECT 1 FROM information_schema.columns
        WHERE table_name = 'user_facts' AND column_name = 'predicate'
   );

ALTER TABLE user_facts ALTER COLUMN predicate_code SET NOT NULL;
ALTER TABLE user_facts DROP CONSTRAINT IF EXISTS user_facts_predicate_code_check;
ALTER TABLE user_facts ADD CONSTRAINT user_facts_predicate_code_check CHECK (predicate_code IN (
    'training_for', 'working_toward', 'target_race', 'prefer', 'avoid', 'primarily_train',
        'have_baseline', 'have', 'recovering_from', 'can_train_on', 'cannot_train_on',
        'need_session_on', 'unavailable', 'own', 'train_on', 'train_because', 'parq_yes',
        'flagged', 'states'
));

ALTER TABLE user_facts DROP COLUMN IF EXISTS subject;
ALTER TABLE user_facts DROP COLUMN IF EXISTS predicate;
