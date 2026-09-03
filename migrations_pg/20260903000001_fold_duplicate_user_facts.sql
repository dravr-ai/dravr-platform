-- ABOUTME: Folds facts an athlete already had stored more than once into the row they restate
-- ABOUTME: Exact repeats only — same kind, predicate code and normalised object; paraphrases need embeddings

-- Merging landed on the live path in 31eedd624, but it only stops NEW
-- duplicates. Dev already held 68 rows across 25 exact-duplicate groups:
-- "have connected Strava" six times, "uses WHOOP" four, "are racing a
-- criterium on July 28" three. This folds those, once.
--
-- Deliberately the deterministic half only. A paraphrase — the case that
-- filed carnet#194, where one goal arrived in French and twice in English —
-- can only be matched by embedding similarity, which is not something SQL
-- can do; `pierre-cli harness dedup-memory` covers it per athlete.
--
-- The anchor is the athlete's own words: an onboarding row wins, then the
-- oldest. It keeps its object text and takes the group's highest confidence,
-- so a rewording can never overwrite what the athlete said or lower its
-- confidence. Re-runnable: once a group is folded there is nothing left to
-- match.

WITH normalised AS (
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
        btrim(regexp_replace(lower(object), '\s+', ' ', 'g'), ' .!?,;:') AS norm
    FROM user_facts
),
grouped AS (
    SELECT
        normalised.*,
        first_value(id) OVER w AS anchor_id,
        count(*) OVER w AS copies
    FROM normalised
    WINDOW w AS (
        PARTITION BY user_id, tenant_id, kind, predicate_code, norm
        ORDER BY (source <> 'onboarding'), created_at, id
        ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING
    )
),
folded AS (
    SELECT
        anchor_id,
        max(confidence) AS best_confidence,
        (array_agg(source_msg_id ORDER BY created_at DESC)
            FILTER (WHERE source_msg_id IS NOT NULL))[1] AS newest_source_msg_id
    FROM grouped
    WHERE copies > 1
    GROUP BY anchor_id
)
UPDATE user_facts AS f
SET confidence = GREATEST(f.confidence, folded.best_confidence),
    source_msg_id = COALESCE(folded.newest_source_msg_id, f.source_msg_id),
    updated_at = now()
FROM folded
WHERE f.id = folded.anchor_id;

WITH normalised AS (
    SELECT
        id,
        user_id,
        tenant_id,
        kind,
        predicate_code,
        source,
        created_at,
        btrim(regexp_replace(lower(object), '\s+', ' ', 'g'), ' .!?,;:') AS norm
    FROM user_facts
),
grouped AS (
    SELECT
        normalised.*,
        first_value(id) OVER w AS anchor_id,
        count(*) OVER w AS copies
    FROM normalised
    WINDOW w AS (
        PARTITION BY user_id, tenant_id, kind, predicate_code, norm
        ORDER BY (source <> 'onboarding'), created_at, id
        ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING
    )
)
DELETE FROM user_facts
WHERE id IN (SELECT id FROM grouped WHERE copies > 1 AND id <> anchor_id);
