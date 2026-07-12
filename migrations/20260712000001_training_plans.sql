-- ABOUTME: Training plans — coach-authored plan outlines + weekly microcycles (SQLite)
-- ABOUTME: First-class entity for coach prescriptions; supersession chains, never in-place mutation

-- A TRAINING PLAN outline is the coach's strategic answer to a dated goal:
-- the goal-race snapshot, the block structure (base/build/peak/taper), and a
-- prose strategy. Weekly day-by-day detail lives in training_plan_weeks.
-- Plans are captured by explicit coach tool calls (save_training_plan), never
-- by post-hoc extraction — extraction minting coach prescriptions as
-- user_facts is what this table replaces (fact 1b6199d8, 2026-07-10).
--
-- goal_race_json is a SNAPSHOT taken at plan time so the plan stays coherent
-- as an artifact; goal_fact_id links to the pillar `Goal` user_fact that is
-- the living source of truth (goal superseded => plan flagged stale on read).
-- Adjustments are prospective-only whole-row supersession: a new row with
-- supersedes_id set, old row status='superseded'. Rows are never mutated
-- after creation except the status flip.
--
-- tenant_id / user_id / coach_slug / plan ids are TEXT query keys (not FKs)
-- to dodge the VARCHAR-tenant-id-decoded-as-UUID bind trap. Every read is
-- tenant-scoped (WHERE tenant_id = ?). Timestamps are Unix epoch SECONDS;
-- calendar dates (race_date, week_start) are TEXT 'YYYY-MM-DD' — they are
-- civil dates, not instants, and must not shift with timezones.
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
    created_at             INTEGER NOT NULL,
    updated_at             INTEGER NOT NULL
);

-- One ACTIVE outline per (tenant, user, coach) — saving a new one supersedes.
CREATE UNIQUE INDEX IF NOT EXISTS idx_training_plans_one_active
    ON training_plans(tenant_id, user_id, coach_slug) WHERE status = 'active';
-- Retrieval lists a user's plans (active first, then history).
CREATE INDEX IF NOT EXISTS idx_training_plans_lookup
    ON training_plans(tenant_id, user_id, status);

-- A TRAINING PLAN WEEK is one microcycle: seven day rows (date, sport,
-- workout, duration, intensity relative to thresholds — zones/%FTP so an FTP
-- retest never invalidates stored plans). "Move Tuesday to Wednesday" is a
-- whole-week re-save superseding this row; past weeks are immutable and
-- past-ness is derived from week_start at read time (no 'completed' status).
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
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);

-- One ACTIVE row per (tenant, user, plan, week) — adjustment = supersession.
CREATE UNIQUE INDEX IF NOT EXISTS idx_training_plan_weeks_one_active
    ON training_plan_weeks(tenant_id, user_id, plan_id, week_start) WHERE status = 'active';
-- Retrieval walks a plan's active weeks in calendar order.
CREATE INDEX IF NOT EXISTS idx_training_plan_weeks_lookup
    ON training_plan_weeks(tenant_id, user_id, plan_id, status, week_start);
