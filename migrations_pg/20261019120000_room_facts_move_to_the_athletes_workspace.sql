-- ABOUTME: Moves the facts a group room filed under its channel tenant onto each athlete's own workspace
-- ABOUTME: One-way: a restatement folds into its home copy, and copies from several rooms merge into one
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Until carnet#926 a messaging group room stamped every ordinary turn's
-- extracted facts under the room's CONVERSATION tenant — the channel (bot
-- owner's) tenant the webhook carries — while the athlete's walk answers, DM
-- turns, the dossier and stage 7e's OKF bundle all read the athlete's own
-- workspace, the turn's TOOL tenant. A fact said in a room ("I have a knee
-- injury") therefore sat in a tenant the athlete does not belong to and never
-- reached their own turns. The pipeline now stamps every turn kind under the
-- tool tenant; this migration moves the rows written before that.
--
-- The athlete's own workspace is resolved exactly the way the runtime
-- resolves the tool tenant of a messaging turn: authenticate_channel's
-- active_tenant_id = tenants.list_for_user().first() — the EARLIEST
-- tenant_users membership (joined_at ASC) in an ACTIVE tenant that has an
-- owner row (LIST_TENANTS_FOR_USER_SQL). The t.id tie-break only makes a
-- joined_at tie deterministic, as in 20260623000001. A user with no such
-- membership is left untouched: the runtime falls back to the webhook tenant
-- for them too.
--
-- A row moves when it sits outside that workspace AND either
--   * it cites a message of a group-room conversation (chat_conversations
--     .group_id IS NOT NULL) under the row's own tenant — the conversation
--     tenant the room stamped it with; or
--   * the user holds no membership in the row's tenant at all, so no turn of
--     theirs could ever read it there (this keeps the facts of a room whose
--     conversation has since been deleted, which cascades its messages).
-- A fact under another tenant the athlete belongs to, filed outside any room,
-- stays where it is: an in-app turn under that tenant reads it there.
--
-- user_facts has no unique key beyond its id, so a move cannot violate a
-- constraint. The collision that matters is a restatement: the same claim
-- (source, predicate_code, object — DELETE_FACTS_BY_CLAIM_SQL's definition)
-- already on file at home, in force at least as long. That room row is folded
-- into the home row the way MERGE_USER_FACT_SQL folds a restatement into its
-- anchor — the higher confidence wins, first_party_only only tightens, the
-- later updated_at stands — and then deleted.
--
-- The same claim said in the rooms of two or more channel tenants, with no
-- copy at home, would otherwise land home twice, and the live merge never
-- cleans an existing pair. So the moving rows of one claim per (home tenant,
-- user) are merged the same way into ONE survivor: the earliest created_at,
-- the lowest id (byte order, COLLATE "C") breaking a tie — the anchor the
-- live merge would have kept. The survivor also keeps the longest validity
-- (NULL when any copy is open-ended), so a merge never ends a claim earlier
-- than one of its copies did. The rest are deleted. Every remaining row is
-- re-stamped, so at most one in-force row per claim per (home tenant, user)
-- comes out of the move.

CREATE TEMP TABLE IF NOT EXISTS room_fact_moves (
    fact_id VARCHAR(255) PRIMARY KEY,
    home_tenant VARCHAR(255) NOT NULL
);
DELETE FROM room_fact_moves;

-- 1. Which rows move, and where.
WITH home AS (
    SELECT DISTINCT ON (tu.user_id) tu.user_id::text AS user_id, t.id::text AS tenant_id
    FROM tenant_users tu
    JOIN tenants t ON t.id = tu.tenant_id AND t.is_active = true
    JOIN tenant_users owner ON owner.tenant_id = t.id AND owner.role = 'owner'
    ORDER BY tu.user_id, tu.joined_at ASC, t.id ASC
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
          WHERE mine.user_id::text = f.user_id
            AND mine.tenant_id::text = f.tenant_id
      )
  );

-- 2. A restatement folds into the home row it repeats.
WITH restated AS (
    SELECT h.id AS home_id,
           MAX(f.confidence) AS confidence,
           BOOL_OR(f.first_party_only) AS first_party_only,
           MAX(f.updated_at) AS updated_at
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
    GROUP BY h.id
)
UPDATE user_facts h
SET confidence = CASE WHEN h.confidence >= r.confidence THEN h.confidence ELSE r.confidence END,
    first_party_only = h.first_party_only OR r.first_party_only,
    updated_at = CASE WHEN h.updated_at >= r.updated_at THEN h.updated_at ELSE r.updated_at END
FROM restated r
WHERE h.id = r.home_id;

DELETE FROM user_facts f
USING room_fact_moves mv
WHERE f.id = mv.fact_id
  AND EXISTS (
      SELECT 1 FROM user_facts h
      WHERE h.tenant_id = mv.home_tenant
        AND h.user_id = f.user_id
        AND h.source = f.source
        AND h.predicate_code = f.predicate_code
        AND h.object = f.object
        AND (h.valid_until IS NULL
             OR (f.valid_until IS NOT NULL AND h.valid_until >= f.valid_until))
  );

-- 3. Copies of one claim from different rooms merge into one survivor.
CREATE TEMP TABLE IF NOT EXISTS room_fact_merges (
    fact_id VARCHAR(255) PRIMARY KEY,
    survivor_id VARCHAR(255) NOT NULL
);
DELETE FROM room_fact_merges;

WITH ranked AS (
    SELECT f.id,
           FIRST_VALUE(f.id) OVER (
               PARTITION BY mv.home_tenant, f.user_id, f.source, f.predicate_code, f.object
               ORDER BY f.created_at ASC, f.id COLLATE "C" ASC
           ) AS survivor_id
    FROM room_fact_moves mv
    JOIN user_facts f ON f.id = mv.fact_id
)
INSERT INTO room_fact_merges (fact_id, survivor_id)
SELECT id, survivor_id FROM ranked WHERE id <> survivor_id;

WITH merged AS (
    SELECT m.survivor_id,
           MAX(f.confidence) AS confidence,
           BOOL_OR(f.first_party_only) AS first_party_only,
           MAX(f.updated_at) AS updated_at,
           BOOL_OR(f.valid_until IS NULL) AS open_ended,
           MAX(f.valid_until) AS valid_until
    FROM room_fact_merges m
    JOIN user_facts f ON f.id = m.fact_id
    GROUP BY m.survivor_id
)
UPDATE user_facts s
SET confidence = CASE WHEN s.confidence >= m.confidence THEN s.confidence ELSE m.confidence END,
    first_party_only = s.first_party_only OR m.first_party_only,
    updated_at = CASE WHEN s.updated_at >= m.updated_at THEN s.updated_at ELSE m.updated_at END,
    valid_until = CASE
        WHEN s.valid_until IS NULL OR m.open_ended THEN NULL
        WHEN s.valid_until >= m.valid_until THEN s.valid_until
        ELSE m.valid_until
    END
FROM merged m
WHERE s.id = m.survivor_id;

DELETE FROM user_facts
WHERE id IN (SELECT fact_id FROM room_fact_merges);

-- 4. Every other room row is re-stamped under the athlete's own workspace.
UPDATE user_facts f
SET tenant_id = mv.home_tenant
FROM room_fact_moves mv
WHERE f.id = mv.fact_id;

DROP TABLE IF EXISTS room_fact_merges;
DROP TABLE IF EXISTS room_fact_moves;
