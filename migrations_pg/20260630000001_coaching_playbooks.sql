-- ABOUTME: Coaching playbook procedural memory — playbooks, pending advice, archetype priors (PostgreSQL)
-- ABOUTME: Mirrors the SQLite migration; all timestamps are BIGINT Unix epoch seconds (integer-pure)

-- See the matching SQLite migration for the full rationale. Column NAMES are
-- identical across backends (schema_parity_test asserts this); only the integer
-- width differs (SQLite INTEGER -> PG BIGINT). `confidence` is intentionally NOT
-- a column: it is the Wilson lower bound, computed in Rust on read. tenant_id /
-- user_id / coach_slug are TEXT (query keys, not FKs) to avoid the VARCHAR-vs-
-- native-UUID bind trap; every read is tenant-scoped except archetype_priors
-- (the documented non-tenant carve-out). All timestamps are Unix epoch seconds.
CREATE TABLE IF NOT EXISTS coaching_playbooks (
    id                  TEXT PRIMARY KEY,
    tenant_id           TEXT NOT NULL,
    user_id             TEXT NOT NULL,
    coach_slug          TEXT NOT NULL DEFAULT '',
    trigger_hash        TEXT NOT NULL,
    intervention_hash   TEXT NOT NULL,
    trigger_json        TEXT NOT NULL,
    intervention_json   TEXT NOT NULL,
    outcome_metric_json TEXT NOT NULL,
    success_count       BIGINT NOT NULL DEFAULT 0,
    failure_count       BIGINT NOT NULL DEFAULT 0,
    neutral_count       BIGINT NOT NULL DEFAULT 0,
    last_outcome_at     BIGINT,
    created_at          BIGINT NOT NULL,
    updated_at          BIGINT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_playbooks_unique
    ON coaching_playbooks(tenant_id, user_id, coach_slug, trigger_hash, intervention_hash);
CREATE INDEX IF NOT EXISTS idx_playbooks_lookup
    ON coaching_playbooks(tenant_id, user_id, coach_slug);

CREATE TABLE IF NOT EXISTS pending_advice (
    id                  TEXT PRIMARY KEY,
    tenant_id           TEXT NOT NULL,
    user_id             TEXT NOT NULL,
    coach_slug          TEXT NOT NULL DEFAULT '',
    playbook_id         TEXT,
    trigger_json        TEXT NOT NULL,
    intervention_json   TEXT NOT NULL,
    outcome_metric_json TEXT NOT NULL,
    baseline_json       TEXT NOT NULL,
    due_by              BIGINT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'pending',
    label               TEXT,
    label_source        TEXT,
    source_msg_id       TEXT,
    created_at          BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_pending_advice_due
    ON pending_advice(status, due_by);
CREATE INDEX IF NOT EXISTS idx_pending_advice_user
    ON pending_advice(tenant_id, user_id);

-- The ONLY non-tenant store: k-anonymous archetype aggregate for cold-start.
-- Counts only, no tenant_id / user_id / free text; rows materialized only once
-- distinct_user_count >= K (enforced at write time). Documented carve-out to the
-- tenant-scoping rule — safe because no row is attributable to a tenant or user.
CREATE TABLE IF NOT EXISTS archetype_priors (
    archetype_key       TEXT NOT NULL,
    trigger_hash        TEXT NOT NULL,
    intervention_hash   TEXT NOT NULL,
    trigger_json        TEXT NOT NULL,
    intervention_json   TEXT NOT NULL,
    success_count       BIGINT NOT NULL DEFAULT 0,
    failure_count       BIGINT NOT NULL DEFAULT 0,
    distinct_user_count BIGINT NOT NULL DEFAULT 0,
    updated_at          BIGINT NOT NULL,
    PRIMARY KEY (archetype_key, trigger_hash, intervention_hash)
);
