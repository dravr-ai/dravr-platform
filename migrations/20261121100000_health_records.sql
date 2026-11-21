-- ABOUTME: health_records — the T2–T3 records an import or a lab document yields: self-reports, clinical records, lab documents (SQLite)
-- ABOUTME: New tables only (carnet#1544, owl phase 5 slice 2): every import stores them for the athlete; the tier consent gates who else reads

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- JF 2026-11-20, "Store, gate reads": every import keeps the T2–T3 records
-- dravr-barrage reads (cycle-tracking self-reports, FHIR R4 clinical
-- records); the athlete's tier consent decides only who else reads them.
-- Self-measured glucose goes to the riviere series store, not here.
--
-- Each record is keyed by the barrage id it came with, per provider key
-- (the import's source key, or lab_document), filed under the data source
-- the import wrote, as every wellness row is, so a
-- re-import replaces its own rows. A clinical record waits for the athlete
-- while review_state = 'pending'; their answer is review_decision, which a
-- re-import never overwrites: a rejected record stays rejected, and no
-- tombstone is needed. lab_documents names each lab PDF the athlete
-- uploaded; its bytes live in the blob store under the upload key, as every
-- uploaded file's do.
--
-- Additive: three new tables, no backfill. Retracting a source deletes its
-- rows (provider_data/statements.rs); the account delete cascades here.

CREATE TABLE IF NOT EXISTS self_reports (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    data_source_id TEXT NOT NULL,
    record_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    start_at TEXT NOT NULL,
    end_at TEXT NOT NULL,
    value_code TEXT,
    value REAL,
    unit TEXT,
    cycle_start INTEGER NOT NULL DEFAULT 0,
    protection_used INTEGER,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (tenant_id, user_id, provider, record_id)
);

CREATE INDEX IF NOT EXISTS idx_self_reports_owner_start
    ON self_reports (tenant_id, user_id, start_at);

CREATE TABLE IF NOT EXISTS lab_documents (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    file_sha256 TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    pages INTEGER NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('read', 'no_text')),
    engine TEXT,
    flagged_lines INTEGER NOT NULL DEFAULT 0,
    skipped INTEGER NOT NULL DEFAULT 0,
    dropped INTEGER NOT NULL DEFAULT 0,
    model_failure TEXT,
    received_at TEXT NOT NULL,
    UNIQUE (tenant_id, user_id, file_sha256)
);

CREATE TABLE IF NOT EXISTS clinical_records (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    data_source_id TEXT NOT NULL,
    record_id TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    tier TEXT NOT NULL CHECK (tier IN ('T0', 'T1', 'T2', 'T3')),
    sensitivity_class TEXT,
    fhir_version TEXT NOT NULL,
    resource TEXT NOT NULL,
    stripped TEXT NOT NULL,
    provenance TEXT NOT NULL,
    flat TEXT,
    derivation TEXT NOT NULL,
    review_state TEXT NOT NULL CHECK (review_state IN ('none', 'pending')),
    review_decision TEXT CHECK (review_decision IN ('confirmed', 'rejected')),
    checks TEXT NOT NULL,
    locator TEXT,
    read_by TEXT,
    effective_at TEXT,
    lab_document_id TEXT REFERENCES lab_documents(id) ON DELETE SET NULL,
    decided_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (tenant_id, user_id, provider, record_id)
);

CREATE INDEX IF NOT EXISTS idx_clinical_records_owner_effective
    ON clinical_records (tenant_id, user_id, effective_at);
