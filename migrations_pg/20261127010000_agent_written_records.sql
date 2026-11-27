-- ABOUTME: Agent-written records (the pending store and its confirmed rows), clinical review logging, and one live child per (parent, scope) (PostgreSQL)
-- ABOUTME: ONE-WAY: creates agent_written_records and recreates idx_oauth_client_grants_live_child, an index the previous binary relies on

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Owl phase 5, slice 4c (carnet#1544, plan revision 21 §21.13). The SQLite
-- twin documents each column; the two schemas match.
CREATE TABLE IF NOT EXISTS agent_written_records (
    id               TEXT        NOT NULL,
    tenant_id        TEXT        NOT NULL,
    user_id          UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    client_grant_id  TEXT        NOT NULL,
    client_id        TEXT        NOT NULL,
    stream           TEXT        NOT NULL,
    tier             TEXT        NOT NULL CHECK (tier IN ('T0', 'T1', 'T2')),
    effective_at     TIMESTAMPTZ,
    fields           TEXT        NOT NULL,
    fhir             TEXT,
    provenance       TEXT        NOT NULL,
    review_state     TEXT        NOT NULL CHECK (review_state IN ('pending', 'confirmed', 'rejected')),
    decided_at       TIMESTAMPTZ,
    created_at       TIMESTAMPTZ NOT NULL,
    ingest_logged_at TIMESTAMPTZ,
    review_logged_at TIMESTAMPTZ,
    attempts         INTEGER     NOT NULL DEFAULT 0,
    poisoned_at      TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, user_id, id)
);
CREATE INDEX IF NOT EXISTS idx_agent_written_review
    ON agent_written_records (user_id, tenant_id, review_state, created_at);
CREATE INDEX IF NOT EXISTS idx_agent_written_served
    ON agent_written_records (user_id, tenant_id, stream, effective_at)
    WHERE review_state = 'confirmed';

ALTER TABLE clinical_records ADD COLUMN IF NOT EXISTS review_logged_at TIMESTAMPTZ;

DROP INDEX IF EXISTS idx_oauth_client_grants_live_child;
CREATE UNIQUE INDEX IF NOT EXISTS idx_oauth_client_grants_live_child
    ON oauth_client_grants (parent_grant_id, scope)
    WHERE revoked_at IS NULL AND parent_grant_id IS NOT NULL;
