-- ABOUTME: Adds the two season-horizon goal codes to user_facts.predicate_code's closed vocabulary
-- ABOUTME: The Postgres half: the CHECK constraint is replaced in place; every statement is re-runnable

-- The /season walk asks what "good" looks like this season and, separately,
-- in a few years, and files each under its own code — aim_this_season and
-- aim_long_term — because about_you supersedes every 'working_toward' fact on
-- each write, and two horizons under one code would clobber each other.
ALTER TABLE user_facts DROP CONSTRAINT IF EXISTS user_facts_predicate_code_check;
ALTER TABLE user_facts ADD CONSTRAINT user_facts_predicate_code_check CHECK (predicate_code IN (
    'training_for', 'working_toward', 'target_race', 'aim_this_season', 'aim_long_term',
    'prefer', 'avoid', 'primarily_train',
    'have_baseline', 'have', 'recovering_from', 'can_train_on', 'cannot_train_on',
    'need_session_on', 'unavailable', 'own', 'train_on', 'train_because', 'parq_yes',
    'flagged', 'states'
));
