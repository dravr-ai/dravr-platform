-- ABOUTME: Agent-written records (the pending store and its confirmed rows), clinical review logging, and one live child per (parent, scope) (SQLite)
-- ABOUTME: ONE-WAY: creates agent_written_records and recreates idx_oauth_client_grants_live_child, an index the previous binary relies on

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Owl phase 5, slice 4c (carnet#1544, plan revision 21 §21.13).
--
-- agent_written_records: every record an outside client PROPOSED under an
-- agent-write grant (owl PDPP §4.5, DM §2.5). Its authorship never changes;
-- only review_state moves, pending -> confirmed | rejected, once. Owl's
-- health.vault.agent.pending is the pending subset, never readable through a
-- grant; a confirmed row is served beside its stream, labelled untrusted.
--   id                barrage agent_record_id: keyed over (client, stream,
--                     canonical row), so an identical resend is the same row
--   client_grant_id   the agent-write grant it was written under; no FK, so
--                     a hard-deleted grant keeps its records
--   client_id         the registered client that proposed it
--   stream            owl's name without the health.vault. prefix
--   tier              the stream's tier: T0 to T2, never T3
--   effective_at      the row's time; NULL for profile.summary
--   fields            the canonical JSON row
--   fhir              the built FHIR resource, for the clinical-shaped rows
--   provenance        owl's envelope (source agent, lane AG, legal basis …)
--   decided_at        when the athlete confirmed or rejected it
--   ingest_logged_at, review_logged_at, attempts, poisoned_at
--                     the record-log outbox, as oauth_grant_events has it
CREATE TABLE IF NOT EXISTS agent_written_records (
    id               TEXT    NOT NULL,
    tenant_id        TEXT    NOT NULL,
    user_id          TEXT    NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    client_grant_id  TEXT    NOT NULL,
    client_id        TEXT    NOT NULL,
    stream           TEXT    NOT NULL,
    tier             TEXT    NOT NULL CHECK (tier IN ('T0', 'T1', 'T2')),
    effective_at     TEXT,
    fields           TEXT    NOT NULL,
    fhir             TEXT,
    provenance       TEXT    NOT NULL,
    review_state     TEXT    NOT NULL CHECK (review_state IN ('pending', 'confirmed', 'rejected')),
    decided_at       TEXT,
    created_at       TEXT    NOT NULL,
    ingest_logged_at TEXT,
    review_logged_at TEXT,
    attempts         INTEGER NOT NULL DEFAULT 0,
    poisoned_at      TEXT,
    PRIMARY KEY (tenant_id, user_id, id)
);
CREATE INDEX IF NOT EXISTS idx_agent_written_review
    ON agent_written_records (user_id, tenant_id, review_state, created_at);
CREATE INDEX IF NOT EXISTS idx_agent_written_served
    ON agent_written_records (user_id, tenant_id, stream, effective_at)
    WHERE review_state = 'confirmed';

-- clinical_records: when the record-log worker logged a decided row's
-- `review` entry (DM §2.5 records every review_state change); NULL until
-- then, and for every undecided row.
ALTER TABLE clinical_records ADD COLUMN review_logged_at TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- One live child per (parent, scope): a parent holds one step-up child
-- (scope `<parent> pdpp:preset:one_off_question`) and one agent-write child
-- (scope `pdpp:agent_write`) at once. 20261126200000 held it to one child of
-- any kind. A previous binary approving a step-up still revokes every live
-- child of the parent first, an agent-write one included: the host steps up
-- again, and no record is lost.
DROP INDEX IF EXISTS idx_oauth_client_grants_live_child;
CREATE UNIQUE INDEX IF NOT EXISTS idx_oauth_client_grants_live_child
    ON oauth_client_grants (parent_grant_id, scope)
    WHERE revoked_at IS NULL AND parent_grant_id IS NOT NULL;
