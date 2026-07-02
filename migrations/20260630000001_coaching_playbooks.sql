-- ABOUTME: Coaching playbook procedural memory — playbooks, pending advice, archetype priors (SQLite)
-- ABOUTME: The policy/learning layer over the capability state model (see ADR-007). Outcome-reinforced.

-- A coaching PLAYBOOK is a learned `trigger -> intervention` pair plus the
-- reinforcement counters that say how well it has worked for this athlete.
-- `confidence` is NOT stored: it is the Wilson lower bound of the success rate,
-- computed in Rust on read (single source of truth in pierre-memory) so the
-- counter writes can be a single atomic ON CONFLICT increment with no recompute.
--
-- `coach_slug` is NOT NULL DEFAULT '' (empty string = coach-agnostic) rather than
-- nullable, so the uniqueness key below actually constrains coach-agnostic rows
-- (NULLs compare distinct and would let duplicates accumulate). The repository
-- maps Option<String> <-> '' at the boundary.
-- tenant_id / user_id / coach_slug are TEXT on both backends (query keys, not FKs)
-- to dodge the VARCHAR-tenant-id-decoded-as-UUID bind trap. Every read is
-- tenant-scoped (WHERE tenant_id = ?).
-- All timestamps are Unix epoch SECONDS (integers) so reads/writes are identical
-- across SQLite and PostgreSQL with no datetime-format ambiguity.
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
    success_count       INTEGER NOT NULL DEFAULT 0,
    failure_count       INTEGER NOT NULL DEFAULT 0,
    neutral_count       INTEGER NOT NULL DEFAULT 0,
    last_outcome_at     INTEGER,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL
);

-- One playbook per (tenant, user, coach, trigger, intervention) — the ON CONFLICT
-- target for the atomic counter upsert.
CREATE UNIQUE INDEX IF NOT EXISTS idx_playbooks_unique
    ON coaching_playbooks(tenant_id, user_id, coach_slug, trigger_hash, intervention_hash);
-- Retrieval lists a user's playbooks (optionally scoped to a coach).
CREATE INDEX IF NOT EXISTS idx_playbooks_lookup
    ON coaching_playbooks(tenant_id, user_id, coach_slug);

-- PENDING ADVICE is an in-flight recommendation awaiting its observed outcome.
-- The evaluator scans `status='pending' AND due_by <= now`, reads the data window,
-- labels it, and rolls the result into a playbook's counters.
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
    due_by              INTEGER NOT NULL,
    status              TEXT NOT NULL DEFAULT 'pending',
    label               TEXT,
    label_source        TEXT,
    source_msg_id       TEXT,
    created_at          INTEGER NOT NULL
);

-- The evaluator's hot scan: due, still-pending advice.
CREATE INDEX IF NOT EXISTS idx_pending_advice_due
    ON pending_advice(status, due_by);
CREATE INDEX IF NOT EXISTS idx_pending_advice_user
    ON pending_advice(tenant_id, user_id);

-- ARCHETYPE PRIORS is the ONLY intentionally NON-tenant store: a k-anonymous
-- aggregate of which interventions work for an athlete archetype, used purely
-- for cold-start (a new user inherits priors for their archetype, then
-- personalizes). It holds COUNTS only — no tenant_id, no user_id, no free text —
-- and a row is only materialized once `distinct_user_count` >= K (enforced at
-- write time in the aggregation job). This is the documented carve-out to the
-- "every query is tenant-scoped" rule; it is safe because no row is attributable
-- to a tenant or user.
CREATE TABLE IF NOT EXISTS archetype_priors (
    archetype_key       TEXT NOT NULL,
    trigger_hash        TEXT NOT NULL,
    intervention_hash   TEXT NOT NULL,
    trigger_json        TEXT NOT NULL,
    intervention_json   TEXT NOT NULL,
    success_count       INTEGER NOT NULL DEFAULT 0,
    failure_count       INTEGER NOT NULL DEFAULT 0,
    distinct_user_count INTEGER NOT NULL DEFAULT 0,
    updated_at          INTEGER NOT NULL,
    PRIMARY KEY (archetype_key, trigger_hash, intervention_hash)
);
