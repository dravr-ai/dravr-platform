-- ABOUTME: A human coach's training-plan draft lives beside the athlete's live season, unseen until published (carnet#1069)
-- ABOUTME: Rebuilds training_plans (SQLite) for three statuses, three draft columns, one open draft per coach and athlete
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A group's human coach writes a coached athlete's plan into a draft the
-- athlete never sees, then publishes it in one step or throws it away. The
-- draft is a row of this table in status 'draft', under the athlete's tenant
-- and user like their live season, and every athlete read selects status
-- 'active' only, so a draft reaches no surface of theirs.
--
-- The draft is an overlay, not a copy of the season. Its row holds an
-- outline: the one the coach re-laid (outline_drafted = 1), or the live one
-- copied verbatim (outline_drafted = 0) when the coach changes weeks only.
-- Only the weeks the coach saved hang off it in training_plan_weeks. Publish
-- replays it through the live save path and marks the row 'published';
-- discard marks it 'discarded'. Neither row is read by an athlete surface.
--
--   drafted_by_user_id  the coach who owns the draft: a users(id) as text,
--                       like user_id, and required on a draft row.
--   draft_base_plan_id  the live season the draft began from, NULL when the
--                       athlete had none. A live season that no longer has
--                       this id was re-laid under the draft.
--   outline_drafted     whether publishing re-lays the season, or only puts
--                       the drafted weeks on the live one.
--
-- SQLite cannot change a CHECK in place, so the table is rebuilt with every
-- column, default and index it carries (the July table plus the vision,
-- one-season and provenance migrations), and every row is copied as it is.
-- No foreign key points at training_plans, and none leaves it.
CREATE TABLE training_plans_new ( -- idempotency-ok: table rebuild — SQLite cannot change a CHECK in place; sqlx applies each migration exactly once
    id                     TEXT PRIMARY KEY,
    tenant_id              TEXT NOT NULL,
    user_id                TEXT NOT NULL,
    author_agent_id        TEXT NOT NULL DEFAULT '',
    goal_fact_id           TEXT,
    goal_race_json         TEXT NOT NULL,
    races_json             TEXT NOT NULL DEFAULT '[]',
    strategy               TEXT NOT NULL,
    phases_json            TEXT NOT NULL,
    status                 TEXT NOT NULL DEFAULT 'active'
                           CHECK (status IN ('active','superseded','completed','abandoned',
                                             'draft','published','discarded')),
    supersedes_id          TEXT,
    source_conversation_id TEXT,
    created_at             INTEGER NOT NULL,
    updated_at             INTEGER NOT NULL,
    flavour_json           TEXT,
    season_start           TEXT,
    season_end             TEXT,
    first_party_only       INTEGER NOT NULL DEFAULT 0,
    drafted_by_user_id     TEXT,
    draft_base_plan_id     TEXT,
    outline_drafted        INTEGER NOT NULL DEFAULT 0,
    CHECK (status <> 'draft' OR drafted_by_user_id IS NOT NULL)
);
INSERT INTO training_plans_new (
    id, tenant_id, user_id, author_agent_id, goal_fact_id, goal_race_json, races_json,
    strategy, phases_json, status, supersedes_id, source_conversation_id, created_at,
    updated_at, flavour_json, season_start, season_end, first_party_only
)
  SELECT id, tenant_id, user_id, author_agent_id, goal_fact_id, goal_race_json, races_json,
         strategy, phases_json, status, supersedes_id, source_conversation_id, created_at,
         updated_at, flavour_json, season_start, season_end, first_party_only
  FROM training_plans;
DROP TABLE training_plans; -- idempotency-ok: table rebuild — the rebuilt table is renamed over it on the next line
ALTER TABLE training_plans_new RENAME TO training_plans;

-- Retrieval lists a user's plans (active first, then history).
CREATE INDEX IF NOT EXISTS idx_training_plans_lookup
    ON training_plans(tenant_id, user_id, status);
-- One ACTIVE outline per (tenant, user): the athlete's season.
CREATE UNIQUE INDEX IF NOT EXISTS idx_training_plans_one_season
    ON training_plans(tenant_id, user_id) WHERE status = 'active';
-- One OPEN draft per coach per athlete: every save the coach makes lands on it.
CREATE UNIQUE INDEX IF NOT EXISTS idx_training_plans_one_draft
    ON training_plans(tenant_id, user_id, drafted_by_user_id) WHERE status = 'draft';
