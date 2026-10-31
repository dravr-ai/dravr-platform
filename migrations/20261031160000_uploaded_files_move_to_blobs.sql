-- ABOUTME: Uploaded files move to the blob store: the row keeps the file's metadata, its bytes drain out (SQLite)
-- ABOUTME: Mirrors migrations_pg/20261031160000; SQLite rebuilds the table to let file_bytes be NULL

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the Postgres migration of the same version. SQLite cannot drop a NOT
-- NULL constraint in place, so the table is rebuilt with every row it holds.

CREATE TABLE IF NOT EXISTS uploaded_activity_files_next (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    file_sha256 TEXT NOT NULL,
    byte_size INTEGER NOT NULL,
    file_bytes BLOB,
    uploaded_at TEXT NOT NULL,
    origin TEXT NOT NULL DEFAULT 'upload',
    PRIMARY KEY (tenant_id, user_id, file_sha256)
);

INSERT INTO uploaded_activity_files_next
    (tenant_id, user_id, file_sha256, byte_size, file_bytes, uploaded_at)
SELECT tenant_id, user_id, file_sha256, byte_size, file_bytes, uploaded_at
FROM uploaded_activity_files;

DROP TABLE IF EXISTS uploaded_activity_files;

ALTER TABLE uploaded_activity_files_next RENAME TO uploaded_activity_files;

CREATE INDEX IF NOT EXISTS idx_uploaded_activity_files_user_tenant
    ON uploaded_activity_files(user_id, tenant_id);
