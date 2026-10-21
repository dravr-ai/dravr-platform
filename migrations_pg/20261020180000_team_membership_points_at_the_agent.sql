-- ABOUTME: Marks which agent_assignments rows are team memberships, and folds unedited Store copies back onto their catalogue agent
-- ABOUTME: The PostgreSQL half of the one-way move (carnet#1096); the SQLite half (migrations/, same version) mirrors it statement for statement
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Until carnet#1096 one agent_assignments row meant three things at once: the
-- agent is on the athlete's team, the athlete once used it, and their
-- favourite flag. record_usage, toggle_favorite and activate_agent all created
-- the row, so merely opening a chat with a catalogue agent put it on the team,
-- and a Store install made a frozen personal copy of the system agent instead
-- of pointing at it.
--
-- From here on, "on your team" is a row with is_member = TRUE. A row with
-- is_member = FALSE only carries the usage counters and favourite history.
--
-- 1. is_member is backfilled TRUE only where the row was a real choice: the
--    athlete owns the agent (their own creation, or an installed copy — never
--    a catalogue agent, which the operator owns but only uses), an
--    admin assigned it (assigned_by names someone else), or they favourited
--    it. A row that exists only because the agent was used stays FALSE, and
--    so does the starter agent signup selected for them (carnet#1098: usage
--    is never silently turned into membership).
--
-- 2. A Store copy that nobody ever edited moves back onto its origin. A copy
--    qualifies when it is a non-system agent forked from a SYSTEM agent with
--    no agent_versions row (every athlete edit snapshots one), and either its
--    updated_at still equals created_at (install bound both to one instant)
--    or source = 'contremaitre': before the seeder skipped forks on its slug
--    lookup, a reseed could rewrite an installed copy, bumping updated_at and
--    stamping that source with no athlete involved (an install copy is
--    'custom'; only the seeder writes 'contremaitre').
--    A copy of an agent another tenant's author published stays a copy: that
--    is the reviewed snapshot the athlete installed.
--
--    For each moved copy, every row that points at it is re-pointed at the
--    origin before the copy is deleted, so nothing the athlete built with it
--    (conversations, notes, follow-ups, sessions, plans, facts) is lost. The
--    assignment rows of one (origin, athlete) — the copy's row, the origin's
--    own usage row if any, and the rows of further copies the athlete
--    installed in other tenants — collapse into ONE survivor: the origin's
--    existing row when there is one, else the copy row with the lowest id. It
--    keeps the summed use_count, the OR of is_favorite and the latest
--    last_used_at, and it is a member exactly when one of the folded rows
--    was: the copy owner's row is (step 1), while a teammate's usage-only
--    row on the copy stays usage (carnet#1098). Correlated subqueries rather
--    than UPDATE … FROM, so the two halves read the same.

ALTER TABLE agent_assignments ADD COLUMN IF NOT EXISTS is_member BOOLEAN NOT NULL DEFAULT FALSE;

-- 1. Which existing rows are memberships.
UPDATE agent_assignments
SET is_member = TRUE
WHERE EXISTS (
        SELECT 1 FROM agents a
        WHERE a.id = agent_assignments.agent_id
          AND a.user_id = agent_assignments.user_id
          AND a.is_system = FALSE
    )
   OR (assigned_by IS NOT NULL AND assigned_by <> user_id)
   OR is_favorite = TRUE;

-- 2. Which copies move, and onto which origin.
CREATE TEMP TABLE IF NOT EXISTS agent_copy_moves (
    copy_id VARCHAR(255) PRIMARY KEY,
    origin_id VARCHAR(255) NOT NULL
);
DELETE FROM agent_copy_moves;

INSERT INTO agent_copy_moves (copy_id, origin_id)
SELECT c.id, c.forked_from
FROM agents c
JOIN agents o ON o.id = c.forked_from
WHERE c.forked_from IS NOT NULL
  AND c.is_system = FALSE
  AND (c.updated_at = c.created_at OR c.source = 'contremaitre')
  AND o.is_system = TRUE
  AND NOT EXISTS (SELECT 1 FROM agent_versions v WHERE v.agent_id = c.id);

-- 2a. Memberships: every assignment row on a moving copy, by (origin, athlete).
CREATE TEMP TABLE IF NOT EXISTS agent_copy_rows (
    row_id VARCHAR(255) PRIMARY KEY,
    origin_id VARCHAR(255) NOT NULL,
    user_id VARCHAR(255) NOT NULL
);
DELETE FROM agent_copy_rows;

INSERT INTO agent_copy_rows (row_id, origin_id, user_id)
SELECT aa.id, mv.origin_id, aa.user_id::text
FROM agent_assignments aa
JOIN agent_copy_moves mv ON mv.copy_id = aa.agent_id;

-- The one row each (origin, athlete) keeps.
CREATE TEMP TABLE IF NOT EXISTS agent_membership_survivors (
    origin_id VARCHAR(255) NOT NULL,
    user_id VARCHAR(255) NOT NULL,
    survivor_id VARCHAR(255) NOT NULL,
    PRIMARY KEY (origin_id, user_id)
);
DELETE FROM agent_membership_survivors;

INSERT INTO agent_membership_survivors (origin_id, user_id, survivor_id)
SELECT r.origin_id, r.user_id, COALESCE(
    (SELECT held.id FROM agent_assignments held
     WHERE held.agent_id = r.origin_id AND held.user_id::text = r.user_id),
    MIN(r.row_id)
)
FROM agent_copy_rows r
GROUP BY r.origin_id, r.user_id;

-- Every row folded into a survivor, the survivor itself included.
CREATE TEMP TABLE IF NOT EXISTS agent_membership_folds (
    row_id VARCHAR(255) PRIMARY KEY,
    survivor_id VARCHAR(255) NOT NULL
);
DELETE FROM agent_membership_folds;

INSERT INTO agent_membership_folds (row_id, survivor_id)
SELECT r.row_id, s.survivor_id
FROM agent_copy_rows r
JOIN agent_membership_survivors s ON s.origin_id = r.origin_id AND s.user_id = r.user_id;

INSERT INTO agent_membership_folds (row_id, survivor_id)
SELECT s.survivor_id, s.survivor_id
FROM agent_membership_survivors s
WHERE s.survivor_id NOT IN (SELECT row_id FROM agent_copy_rows);

UPDATE agent_assignments
SET is_member = (
        SELECT BOOL_OR(a.is_member) FROM agent_membership_folds f
        JOIN agent_assignments a ON a.id = f.row_id
        WHERE f.survivor_id = agent_assignments.id
    ),
    use_count = (
        SELECT SUM(a.use_count)::integer FROM agent_membership_folds f
        JOIN agent_assignments a ON a.id = f.row_id
        WHERE f.survivor_id = agent_assignments.id
    ),
    is_favorite = (
        SELECT BOOL_OR(a.is_favorite) FROM agent_membership_folds f
        JOIN agent_assignments a ON a.id = f.row_id
        WHERE f.survivor_id = agent_assignments.id
    ),
    last_used_at = (
        SELECT MAX(a.last_used_at) FROM agent_membership_folds f
        JOIN agent_assignments a ON a.id = f.row_id
        WHERE f.survivor_id = agent_assignments.id
    )
WHERE id IN (SELECT survivor_id FROM agent_membership_survivors);

DELETE FROM agent_assignments
WHERE id IN (SELECT row_id FROM agent_membership_folds)
  AND id NOT IN (SELECT survivor_id FROM agent_membership_survivors);

-- A survivor that was a copy row now points at the origin; the athlete held
-- no origin row, so UNIQUE(agent_id, user_id) cannot collide.
UPDATE agent_assignments
SET agent_id = (
    SELECT s.origin_id FROM agent_membership_survivors s
    WHERE s.survivor_id = agent_assignments.id
)
WHERE id IN (SELECT survivor_id FROM agent_membership_survivors)
  AND agent_id IN (SELECT copy_id FROM agent_copy_moves);

-- 2b. Hide/show preferences: the origin's own row wins, else the lowest-id
--     copy row moves; the rest go.
CREATE TEMP TABLE IF NOT EXISTS agent_preference_moves (
    pref_id VARCHAR(255) PRIMARY KEY,
    origin_id VARCHAR(255) NOT NULL
);
DELETE FROM agent_preference_moves;

INSERT INTO agent_preference_moves (pref_id, origin_id)
SELECT MIN(p.id), mv.origin_id
FROM user_agent_preferences p
JOIN agent_copy_moves mv ON mv.copy_id = p.agent_id
WHERE NOT EXISTS (
    SELECT 1 FROM user_agent_preferences held
    WHERE held.user_id = p.user_id AND held.agent_id = mv.origin_id
)
GROUP BY mv.origin_id, p.user_id;

UPDATE user_agent_preferences
SET agent_id = (
    SELECT pm.origin_id FROM agent_preference_moves pm
    WHERE pm.pref_id = user_agent_preferences.id
)
WHERE id IN (SELECT pref_id FROM agent_preference_moves);

DELETE FROM user_agent_preferences
WHERE agent_id IN (SELECT copy_id FROM agent_copy_moves);

-- 2c. Every other pointer at a moving copy follows it to the origin.
UPDATE chat_conversations
SET agent_id = (SELECT mv.origin_id FROM agent_copy_moves mv WHERE mv.copy_id = chat_conversations.agent_id)
WHERE agent_id IN (SELECT copy_id FROM agent_copy_moves);

UPDATE coaching_groups
SET agent_id = (SELECT mv.origin_id FROM agent_copy_moves mv WHERE mv.copy_id = coaching_groups.agent_id)
WHERE agent_id IN (SELECT copy_id FROM agent_copy_moves);

UPDATE tenant_users
SET selected_agent_id = (SELECT mv.origin_id FROM agent_copy_moves mv WHERE mv.copy_id = tenant_users.selected_agent_id)
WHERE selected_agent_id IN (SELECT copy_id FROM agent_copy_moves);

UPDATE user_facts
SET agent_id = (SELECT mv.origin_id FROM agent_copy_moves mv WHERE mv.copy_id = user_facts.agent_id)
WHERE agent_id IN (SELECT copy_id FROM agent_copy_moves);

UPDATE claim_verdicts
SET agent_id = (SELECT mv.origin_id FROM agent_copy_moves mv WHERE mv.copy_id = claim_verdicts.agent_id)
WHERE agent_id IN (SELECT copy_id FROM agent_copy_moves);

UPDATE athlete_commitments
SET agent_id = (SELECT mv.origin_id FROM agent_copy_moves mv WHERE mv.copy_id = athlete_commitments.agent_id)
WHERE agent_id IN (SELECT copy_id FROM agent_copy_moves);

UPDATE training_plans
SET author_agent_id = (SELECT mv.origin_id FROM agent_copy_moves mv WHERE mv.copy_id = training_plans.author_agent_id)
WHERE author_agent_id IN (SELECT copy_id FROM agent_copy_moves);

UPDATE training_plan_weeks
SET author_agent_id = (SELECT mv.origin_id FROM agent_copy_moves mv WHERE mv.copy_id = training_plan_weeks.author_agent_id)
WHERE author_agent_id IN (SELECT copy_id FROM agent_copy_moves);

UPDATE prescribed_workouts
SET agent_id = (SELECT mv.origin_id FROM agent_copy_moves mv WHERE mv.copy_id = prescribed_workouts.agent_id)
WHERE agent_id IN (SELECT copy_id FROM agent_copy_moves);

UPDATE agent_sessions
SET agent_id = (SELECT mv.origin_id FROM agent_copy_moves mv WHERE mv.copy_id = agent_sessions.agent_id)
WHERE agent_id IN (SELECT copy_id FROM agent_copy_moves);

UPDATE agent_notes
SET agent_id = (SELECT mv.origin_id FROM agent_copy_moves mv WHERE mv.copy_id = agent_notes.agent_id)
WHERE agent_id IN (SELECT copy_id FROM agent_copy_moves);

UPDATE agent_followups
SET agent_id = (SELECT mv.origin_id FROM agent_copy_moves mv WHERE mv.copy_id = agent_followups.agent_id)
WHERE agent_id IN (SELECT copy_id FROM agent_copy_moves);

-- usage_counters.agent_id is a UUID column here (20260424000002).
UPDATE usage_counters
SET agent_id = (SELECT mv.origin_id::uuid FROM agent_copy_moves mv WHERE mv.copy_id = usage_counters.agent_id::text)
WHERE agent_id::text IN (SELECT copy_id FROM agent_copy_moves);

-- agent_introductions is keyed (tenant_id, thread_id, agent_id): a thread that
-- already met the origin keeps that row, and the copy's rows go.
INSERT INTO agent_introductions (tenant_id, thread_id, agent_id, created_at)
SELECT i.tenant_id, i.thread_id, mv.origin_id, MIN(i.created_at)
FROM agent_introductions i
JOIN agent_copy_moves mv ON mv.copy_id = i.agent_id
WHERE i.agent_id IS NOT NULL
GROUP BY i.tenant_id, i.thread_id, mv.origin_id
ON CONFLICT DO NOTHING;

DELETE FROM agent_introductions
WHERE agent_id IN (SELECT copy_id FROM agent_copy_moves);

-- agent_relations is UNIQUE(agent_id, related_agent_id, relation_type) and
-- cascades with either agent: a relation the seeder wrote on the copy, or
-- another agent's relation that resolved the copy by slug, follows it to the
-- origin. One row per resulting relation moves (the lowest id), none when
-- the origin already holds it or it would relate the origin to itself;
-- whatever is left goes with the copy.
CREATE TEMP TABLE IF NOT EXISTS agent_relation_moves (
    relation_id VARCHAR(255) PRIMARY KEY,
    agent_id VARCHAR(255) NOT NULL,
    related_agent_id VARCHAR(255) NOT NULL
);
DELETE FROM agent_relation_moves;

INSERT INTO agent_relation_moves (relation_id, agent_id, related_agent_id)
SELECT MIN(t.id), t.new_agent_id, t.new_related_id
FROM (
    SELECT r.id, r.relation_type,
        COALESCE((SELECT mv.origin_id FROM agent_copy_moves mv WHERE mv.copy_id = r.agent_id), r.agent_id) AS new_agent_id,
        COALESCE((SELECT mv.origin_id FROM agent_copy_moves mv WHERE mv.copy_id = r.related_agent_id), r.related_agent_id) AS new_related_id
    FROM agent_relations r
    WHERE r.agent_id IN (SELECT copy_id FROM agent_copy_moves)
       OR r.related_agent_id IN (SELECT copy_id FROM agent_copy_moves)
) t
WHERE t.new_agent_id <> t.new_related_id
  AND NOT EXISTS (
      SELECT 1 FROM agent_relations held
      WHERE held.agent_id = t.new_agent_id
        AND held.related_agent_id = t.new_related_id
        AND held.relation_type = t.relation_type
  )
GROUP BY t.new_agent_id, t.new_related_id, t.relation_type;

UPDATE agent_relations
SET agent_id = (SELECT rm.agent_id FROM agent_relation_moves rm WHERE rm.relation_id = agent_relations.id),
    related_agent_id = (SELECT rm.related_agent_id FROM agent_relation_moves rm WHERE rm.relation_id = agent_relations.id)
WHERE id IN (SELECT relation_id FROM agent_relation_moves);

DELETE FROM agent_relations
WHERE agent_id IN (SELECT copy_id FROM agent_copy_moves)
   OR related_agent_id IN (SELECT copy_id FROM agent_copy_moves);

-- A fork of a moving copy is forked from the origin from now on; the
-- ON DELETE SET NULL would otherwise cut it loose, and its uninstall would
-- then refuse it as "not installed from the Store".
UPDATE agents
SET forked_from = (SELECT mv.origin_id FROM agent_copy_moves mv WHERE mv.copy_id = agents.forked_from)
WHERE forked_from IN (SELECT copy_id FROM agent_copy_moves);

-- A listing the seeder wrote on a copy becomes the origin's when the origin
-- has none (store_listings is UNIQUE(agent_id)), so the catalogue agent
-- keeps its Store presence; otherwise it cascades with the copy.
UPDATE store_listings
SET agent_id = (SELECT mv.origin_id FROM agent_copy_moves mv WHERE mv.copy_id = store_listings.agent_id)
WHERE agent_id IN (SELECT copy_id FROM agent_copy_moves)
  AND NOT EXISTS (
      SELECT 1 FROM store_listings held
      WHERE held.agent_id = (SELECT mv.origin_id FROM agent_copy_moves mv WHERE mv.copy_id = store_listings.agent_id)
  )
  AND store_listings.id = (
      SELECT MIN(sibling.id) FROM store_listings sibling
      JOIN agent_copy_moves smv ON smv.copy_id = sibling.agent_id
      WHERE smv.origin_id = (SELECT mv.origin_id FROM agent_copy_moves mv WHERE mv.copy_id = store_listings.agent_id)
  );

-- 2d. Every listing's install count is re-read from what now holds it, by
--     the rule the running counter keeps from here on: one install per
--     team membership of the agent (step 1 just made favourites and admin
--     assignments memberships, which no counter ever saw), plus one per
--     copy forked from it that stayed (an edited one), each of which was
--     counted when made and is uncounted when uninstalled. The author's own
--     row on an agent they wrote for themselves is not an install: it was
--     there before the listing was (a catalogue agent's operator joins its
--     team by installing it, like anyone). The moved copies are deleted
--     below.
UPDATE store_listings
SET install_count = (
    SELECT COUNT(*) FROM agent_assignments aa
    WHERE aa.agent_id = store_listings.agent_id AND aa.is_member = TRUE
      AND NOT EXISTS (
          SELECT 1 FROM agents own
          WHERE own.id = aa.agent_id AND own.user_id = aa.user_id
            AND own.is_system = FALSE
      )
) + (
    SELECT COUNT(*) FROM agents copy
    WHERE copy.forked_from = store_listings.agent_id
      AND copy.id NOT IN (SELECT copy_id FROM agent_copy_moves)
);

-- 2e. The copies themselves.
DELETE FROM agents
WHERE id IN (SELECT copy_id FROM agent_copy_moves);

DROP TABLE IF EXISTS agent_relation_moves;
DROP TABLE IF EXISTS agent_preference_moves;
DROP TABLE IF EXISTS agent_membership_folds;
DROP TABLE IF EXISTS agent_membership_survivors;
DROP TABLE IF EXISTS agent_copy_rows;
DROP TABLE IF EXISTS agent_copy_moves;
