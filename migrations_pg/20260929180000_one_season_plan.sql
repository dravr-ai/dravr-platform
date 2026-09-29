-- ABOUTME: One active training plan per athlete: the season every agent reads and adds weeks to (PostgreSQL)
-- ABOUTME: agent_slug becomes author_agent_id, weeks gain their own author, and extra active plans are set aside as abandoned
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the matching SQLite migration for the full rationale. The statements are
-- the same, in the same order: the rename is guarded so a re-run is a no-op,
-- tenant_users keys are UUID here and are compared as text against the plan's
-- TEXT keys, and GREATEST / EXTRACT(EPOCH ...) stand in for SQLite's scalar
-- MAX and strftime.

DROP INDEX IF EXISTS idx_training_plans_one_active;
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = current_schema()
          AND table_name = 'training_plans' AND column_name = 'agent_slug'
    ) THEN
        ALTER TABLE training_plans RENAME COLUMN agent_slug TO author_agent_id;
    END IF;
END $$;
ALTER TABLE training_plan_weeks ADD COLUMN IF NOT EXISTS author_agent_id TEXT NOT NULL DEFAULT '';

UPDATE training_plan_weeks w SET author_agent_id = p.author_agent_id
FROM training_plans p WHERE p.id = w.plan_id;

WITH candidates AS (
    SELECT p.id, p.tenant_id, p.user_id,
           CASE WHEN p.author_agent_id <> '' AND p.author_agent_id = tu.selected_agent_id THEN 0
                WHEN p.author_agent_id = '' THEN 1
                ELSE 2 END AS preference,
           GREATEST(p.created_at, COALESCE(wk.last_week, 0)) AS touched
    FROM training_plans p
    LEFT JOIN tenant_users tu ON tu.tenant_id::text = p.tenant_id AND tu.user_id::text = p.user_id
    LEFT JOIN (SELECT plan_id, MAX(created_at) AS last_week FROM training_plan_weeks
               WHERE status = 'active' GROUP BY plan_id) wk ON wk.plan_id = p.id
    WHERE p.status = 'active'),
ranked AS (
    SELECT id, ROW_NUMBER() OVER (PARTITION BY tenant_id, user_id
                                  ORDER BY preference, touched DESC, id) AS season_rank
    FROM candidates)
UPDATE training_plans SET status = 'abandoned', updated_at = EXTRACT(EPOCH FROM NOW())::BIGINT
WHERE id IN (SELECT id FROM ranked WHERE season_rank > 1);

CREATE UNIQUE INDEX IF NOT EXISTS idx_training_plans_one_season
    ON training_plans(tenant_id, user_id) WHERE status = 'active';
