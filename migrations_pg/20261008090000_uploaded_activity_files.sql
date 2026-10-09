-- ABOUTME: uploaded_activity_files — the .fit files athletes uploaded, one row per (tenant, user, file hash) (Postgres)
-- ABOUTME: Mirrors migrations/20261008090000_uploaded_activity_files.sql with BYTEA and PG-native timestamps

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- An athlete can upload the .fit file of a completed workout. Each session in
-- it becomes an activity in cached_activities under the provider key
-- 'upload', so every reader of the cache sees it; the file itself is kept
-- here, because its per-sample series, laps and route are read from it on
-- demand rather than copied into every cached row.
--
-- Keyed by the SHA-256 of the file's bytes, so the same file uploaded twice
-- is one row. Ids are TEXT, like cached_activities, so the statements bind
-- the same way on both engines. The account purge clears the user's rows
-- (user_references.rs).

CREATE TABLE IF NOT EXISTS uploaded_activity_files (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    file_sha256 TEXT NOT NULL,
    byte_size BIGINT NOT NULL,
    file_bytes BYTEA NOT NULL,
    uploaded_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (tenant_id, user_id, file_sha256)
);

CREATE INDEX IF NOT EXISTS idx_uploaded_activity_files_user_tenant
    ON uploaded_activity_files(user_id, tenant_id);
