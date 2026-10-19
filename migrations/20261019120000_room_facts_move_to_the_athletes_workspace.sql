-- ABOUTME: Moves the facts a group room filed under its channel tenant onto each athlete's own workspace
-- ABOUTME: The SQLite half of the one-way move; restatements fold home and copies from several rooms merge into one
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Until carnet#926 a messaging group room stamped every ordinary turn's
-- extracted facts under the room's CONVERSATION tenant — the channel (bot
-- owner's) tenant the webhook carries — while the athlete's walk answers, DM
-- turns, the dossier and stage 7e's OKF bundle all read the athlete's own
-- workspace, the turn's TOOL tenant. The pipeline now stamps every turn kind
-- under the tool tenant; this migration moves the rows written before that.
-- The rule is the Postgres half's (migrations_pg/, same version), which
-- carries the full reasoning:
--
-- * home = the runtime's tool tenant of a messaging turn: the EARLIEST
--   tenant_users membership (joined_at ASC, tenant id breaking a tie) in an
--   ACTIVE tenant that has an owner row; no such membership, no move.
-- * a row outside home moves when it cites a message of a group-room
--   conversation under its own tenant, or when the user holds no membership
--   in its tenant at all.
-- * a restatement of a claim (source, predicate_code, object) already in
--   force at home is folded into the home row — higher confidence,
--   first_party_only only tightens, later updated_at — and deleted.
-- * copies of one claim moving from different rooms, with no home copy to
--   fold into, merge the same way into ONE survivor per (home tenant, user):
--   the earliest created_at, the lowest id (byte order) breaking a tie. It
--   keeps the longest validity (NULL when any copy is open-ended); the rest
--   are deleted.

CREATE TEMP TABLE IF NOT EXISTS room_fact_moves (
    fact_id TEXT PRIMARY KEY,
    home_tenant TEXT NOT NULL
);
DELETE FROM room_fact_moves;

-- 1. Which rows move, and where.
WITH ranked AS (
    SELECT tu.user_id AS user_id, tu.tenant_id AS tenant_id,
           ROW_NUMBER() OVER (
               PARTITION BY tu.user_id
               ORDER BY tu.joined_at ASC, tu.tenant_id ASC
           ) AS rn
    FROM tenant_users tu
    JOIN tenants t ON t.id = tu.tenant_id AND t.is_active = 1
    WHERE EXISTS (
        SELECT 1 FROM tenant_users owner
        WHERE owner.tenant_id = tu.tenant_id AND owner.role = 'owner'
    )
),
home AS (
    SELECT user_id, tenant_id FROM ranked WHERE rn = 1
)
INSERT INTO room_fact_moves (fact_id, home_tenant)
SELECT f.id, home.tenant_id
FROM user_facts f
JOIN home ON home.user_id = f.user_id
WHERE f.tenant_id <> home.tenant_id
  AND (
      EXISTS (
          SELECT 1
          FROM chat_messages m
          JOIN chat_conversations c ON c.id = m.conversation_id
          WHERE m.id = f.source_msg_id
            AND c.group_id IS NOT NULL
            AND c.tenant_id = f.tenant_id
      )
      OR NOT EXISTS (
          SELECT 1 FROM tenant_users mine
          WHERE mine.user_id = f.user_id
            AND mine.tenant_id = f.tenant_id
      )
  );

-- The room rows each resident home row is restated by.
CREATE TEMP TABLE IF NOT EXISTS room_fact_restatements (
    fact_id TEXT PRIMARY KEY,
    home_id TEXT NOT NULL
);
DELETE FROM room_fact_restatements;

INSERT INTO room_fact_restatements (fact_id, home_id)
SELECT f.id, MIN(h.id)
FROM room_fact_moves mv
JOIN user_facts f ON f.id = mv.fact_id
JOIN user_facts h
  ON h.tenant_id = mv.home_tenant
 AND h.user_id = f.user_id
 AND h.source = f.source
 AND h.predicate_code = f.predicate_code
 AND h.object = f.object
 AND (h.valid_until IS NULL
      OR (f.valid_until IS NOT NULL AND h.valid_until >= f.valid_until))
GROUP BY f.id;

-- 2. A restatement folds into the home row it repeats.
UPDATE user_facts
SET confidence = MAX(confidence, (
        SELECT MAX(f.confidence) FROM room_fact_restatements r
        JOIN user_facts f ON f.id = r.fact_id
        WHERE r.home_id = user_facts.id
    )),
    first_party_only = MAX(first_party_only, (
        SELECT MAX(f.first_party_only) FROM room_fact_restatements r
        JOIN user_facts f ON f.id = r.fact_id
        WHERE r.home_id = user_facts.id
    )),
    updated_at = MAX(updated_at, (
        SELECT MAX(f.updated_at) FROM room_fact_restatements r
        JOIN user_facts f ON f.id = r.fact_id
        WHERE r.home_id = user_facts.id
    ))
WHERE id IN (SELECT home_id FROM room_fact_restatements);

DELETE FROM user_facts
WHERE id IN (SELECT fact_id FROM room_fact_restatements);

-- 3. Copies of one claim from different rooms merge into one survivor.
CREATE TEMP TABLE IF NOT EXISTS room_fact_merges (
    fact_id TEXT PRIMARY KEY,
    survivor_id TEXT NOT NULL
);
DELETE FROM room_fact_merges;

WITH ranked AS (
    SELECT f.id AS id,
           FIRST_VALUE(f.id) OVER (
               PARTITION BY mv.home_tenant, f.user_id, f.source, f.predicate_code, f.object
               ORDER BY f.created_at ASC, f.id ASC
           ) AS survivor_id
    FROM room_fact_moves mv
    JOIN user_facts f ON f.id = mv.fact_id
)
INSERT INTO room_fact_merges (fact_id, survivor_id)
SELECT id, survivor_id FROM ranked WHERE id <> survivor_id;

UPDATE user_facts
SET confidence = MAX(confidence, (
        SELECT MAX(f.confidence) FROM room_fact_merges m
        JOIN user_facts f ON f.id = m.fact_id
        WHERE m.survivor_id = user_facts.id
    )),
    first_party_only = MAX(first_party_only, (
        SELECT MAX(f.first_party_only) FROM room_fact_merges m
        JOIN user_facts f ON f.id = m.fact_id
        WHERE m.survivor_id = user_facts.id
    )),
    updated_at = MAX(updated_at, (
        SELECT MAX(f.updated_at) FROM room_fact_merges m
        JOIN user_facts f ON f.id = m.fact_id
        WHERE m.survivor_id = user_facts.id
    )),
    valid_until = CASE
        WHEN valid_until IS NULL OR EXISTS (
            SELECT 1 FROM room_fact_merges m
            JOIN user_facts f ON f.id = m.fact_id
            WHERE m.survivor_id = user_facts.id AND f.valid_until IS NULL
        ) THEN NULL
        ELSE MAX(valid_until, (
            SELECT MAX(f.valid_until) FROM room_fact_merges m
            JOIN user_facts f ON f.id = m.fact_id
            WHERE m.survivor_id = user_facts.id
        ))
    END
WHERE id IN (SELECT survivor_id FROM room_fact_merges);

DELETE FROM user_facts
WHERE id IN (SELECT fact_id FROM room_fact_merges);

-- 4. Every other room row is re-stamped under the athlete's own workspace.
UPDATE user_facts
SET tenant_id = (
    SELECT mv.home_tenant FROM room_fact_moves mv WHERE mv.fact_id = user_facts.id
)
WHERE id IN (SELECT fact_id FROM room_fact_moves);

DROP TABLE IF EXISTS room_fact_merges;
DROP TABLE IF EXISTS room_fact_restatements;
DROP TABLE IF EXISTS room_fact_moves;
