-- ABOUTME: One active training plan per athlete: the season every agent reads and adds weeks to (SQLite)
-- ABOUTME: agent_slug becomes author_agent_id, weeks gain their own author, and extra active plans are set aside as abandoned
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A plan used to be keyed per (tenant, user, agent), so an agent that took
-- over part of a season (a taper, a heat block) could neither read nor adjust
-- the plan the season agent had saved: it built a second plan, and the athlete
-- ended up with two that disagreed. The plan is now the athlete's one season.
-- The agent column no longer selects a plan; it records who laid the outline,
-- and each week records who wrote it, so a handoff shows in the plan itself.
--
-- An athlete who already holds several active plans keeps one of them: the
-- plan their home screen and /plan show today (the selected agent's plan,
-- else the agent-agnostic plan), otherwise the most recently touched one,
-- where a later week counts as a touch, ties broken by id. The others are set
-- to 'abandoned' — nothing wrote that status before this migration, so every
-- abandoned row is one of these — and keep their weeks, so any of them can be
-- restored by flipping the statuses in one transaction.
--
-- Weeks carried from an older outline inherit that outline's author, which is
-- the best record there is of who wrote them.

DROP INDEX IF EXISTS idx_training_plans_one_active;
ALTER TABLE training_plans RENAME COLUMN agent_slug TO author_agent_id;  -- idempotency-ok: SQLite RENAME COLUMN has no IF EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE training_plan_weeks ADD COLUMN author_agent_id TEXT NOT NULL DEFAULT '';  -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

UPDATE training_plan_weeks SET author_agent_id = COALESCE(
    (SELECT p.author_agent_id FROM training_plans p WHERE p.id = training_plan_weeks.plan_id), '');

WITH candidates AS (
    SELECT p.id, p.tenant_id, p.user_id,
           CASE WHEN p.author_agent_id <> '' AND p.author_agent_id = tu.selected_agent_id THEN 0
                WHEN p.author_agent_id = '' THEN 1
                ELSE 2 END AS preference,
           MAX(p.created_at, COALESCE(wk.last_week, 0)) AS touched
    FROM training_plans p
    LEFT JOIN tenant_users tu ON tu.tenant_id = p.tenant_id AND tu.user_id = p.user_id
    LEFT JOIN (SELECT plan_id, MAX(created_at) AS last_week FROM training_plan_weeks
               WHERE status = 'active' GROUP BY plan_id) wk ON wk.plan_id = p.id
    WHERE p.status = 'active'),
ranked AS (
    SELECT id, ROW_NUMBER() OVER (PARTITION BY tenant_id, user_id
                                  ORDER BY preference, touched DESC, id) AS season_rank
    FROM candidates)
UPDATE training_plans SET status = 'abandoned', updated_at = CAST(strftime('%s', 'now') AS INTEGER)
WHERE id IN (SELECT id FROM ranked WHERE season_rank > 1);

-- One ACTIVE outline per (tenant, user): the athlete's season.
CREATE UNIQUE INDEX IF NOT EXISTS idx_training_plans_one_season
    ON training_plans(tenant_id, user_id) WHERE status = 'active';
