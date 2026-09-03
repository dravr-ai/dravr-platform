-- ABOUTME: Folds facts an athlete already had stored more than once into the row they restate
-- ABOUTME: Exact repeats only — same kind, predicate code and normalised object; paraphrases need embeddings

-- The SQLite half of the same one-off cleanup. See the Postgres file for why.
--
-- One difference worth stating rather than hiding: SQLite has no
-- `regexp_replace`, so the normalisation here folds case and trims
-- surrounding whitespace and trailing punctuation, but cannot collapse a
-- double space inside the sentence. Two rows differing only by internal
-- whitespace therefore survive here and fold on Postgres. Dev and production
-- are Postgres; SQLite is the local and test backend, where the pile this
-- migration exists to clear does not accumulate.

WITH grouped AS (
    SELECT
        id,
        anchor_id,
        copies,
        confidence,
        source_msg_id,
        created_at
    FROM (
        SELECT
            id,
            confidence,
            source_msg_id,
            created_at,
            first_value(id) OVER w AS anchor_id,
            count(*) OVER w AS copies
        FROM (
            SELECT
                id,
                user_id,
                tenant_id,
                kind,
                predicate_code,
                confidence,
                source,
                source_msg_id,
                created_at,
                rtrim(ltrim(lower(object)), ' .!?,;:') AS norm
            FROM user_facts
        )
        WINDOW w AS (
            PARTITION BY user_id, tenant_id, kind, predicate_code, norm
            ORDER BY (source <> 'onboarding'), created_at, id
            ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING
        )
    )
    WHERE copies > 1
)
UPDATE user_facts
SET confidence = MAX(
        confidence,
        (SELECT max(confidence) FROM grouped WHERE grouped.anchor_id = user_facts.id)
    ),
    source_msg_id = COALESCE(
        (SELECT source_msg_id FROM grouped
          WHERE grouped.anchor_id = user_facts.id AND grouped.source_msg_id IS NOT NULL
          ORDER BY grouped.created_at DESC LIMIT 1),
        source_msg_id
    ),
    updated_at = CURRENT_TIMESTAMP
WHERE id IN (SELECT anchor_id FROM grouped);

WITH grouped AS (
    SELECT id, anchor_id, copies
    FROM (
        SELECT
            id,
            first_value(id) OVER w AS anchor_id,
            count(*) OVER w AS copies
        FROM (
            SELECT
                id,
                user_id,
                tenant_id,
                kind,
                predicate_code,
                source,
                created_at,
                rtrim(ltrim(lower(object)), ' .!?,;:') AS norm
            FROM user_facts
        )
        WINDOW w AS (
            PARTITION BY user_id, tenant_id, kind, predicate_code, norm
            ORDER BY (source <> 'onboarding'), created_at, id
            ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING
        )
    )
    WHERE copies > 1
)
DELETE FROM user_facts WHERE id IN (SELECT id FROM grouped WHERE id <> anchor_id);
