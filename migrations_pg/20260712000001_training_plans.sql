-- ABOUTME: Training plans — coach-authored plan outlines + weekly microcycles (PostgreSQL)
-- ABOUTME: Mirrors the SQLite migration; timestamps BIGINT Unix epoch seconds, dates TEXT 'YYYY-MM-DD'

-- See the matching SQLite migration for the full rationale. Column NAMES are
-- identical across backends (schema_parity_test asserts this); only the
-- integer width differs (SQLite INTEGER -> PG BIGINT). tenant_id / user_id /
-- coach_slug / plan ids are TEXT query keys (not FKs); every read is
-- tenant-scoped. goal_race_json is a plan-time snapshot; goal_fact_id links
-- the living pillar `Goal` user_fact. Adjustments supersede whole rows,
-- prospective-only; rows are never mutated except the status flip.
CREATE TABLE IF NOT EXISTS training_plans (
    id                     TEXT PRIMARY KEY,
    tenant_id              TEXT NOT NULL,
    user_id                TEXT NOT NULL,
    coach_slug             TEXT NOT NULL DEFAULT '',
    goal_fact_id           TEXT,
    goal_race_json         TEXT NOT NULL,
    races_json             TEXT NOT NULL DEFAULT '[]',
    strategy               TEXT NOT NULL,
    blocks_json            TEXT NOT NULL,
    status                 TEXT NOT NULL DEFAULT 'active'
                           CHECK (status IN ('active','superseded','completed','abandoned')),
    supersedes_id          TEXT,
    source_conversation_id TEXT,
    created_at             BIGINT NOT NULL,
    updated_at             BIGINT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_training_plans_one_active
    ON training_plans(tenant_id, user_id, coach_slug) WHERE status = 'active';
CREATE INDEX IF NOT EXISTS idx_training_plans_lookup
    ON training_plans(tenant_id, user_id, status);

CREATE TABLE IF NOT EXISTS training_plan_weeks (
    id                TEXT PRIMARY KEY,
    tenant_id         TEXT NOT NULL,
    user_id           TEXT NOT NULL,
    plan_id           TEXT NOT NULL,
    week_start        TEXT NOT NULL,
    focus             TEXT NOT NULL DEFAULT '',
    days_json         TEXT NOT NULL,
    status            TEXT NOT NULL DEFAULT 'active'
                      CHECK (status IN ('active','superseded')),
    supersedes_id     TEXT,
    adjustment_reason TEXT NOT NULL DEFAULT '',
    created_at        BIGINT NOT NULL,
    updated_at        BIGINT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_training_plan_weeks_one_active
    ON training_plan_weeks(tenant_id, user_id, plan_id, week_start) WHERE status = 'active';
CREATE INDEX IF NOT EXISTS idx_training_plan_weeks_lookup
    ON training_plan_weeks(tenant_id, user_id, plan_id, status, week_start);
