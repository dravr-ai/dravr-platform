-- ABOUTME: Drops uploaded_activity_files.file_bytes: every uploaded file's bytes live in the blob store (SQLite)
-- ABOUTME: Refuses while any row still holds bytes; mirrors migrations_pg/20261102120000 and rebuilds the table

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the Postgres migration of the same version for why. SQLite has no
-- procedural block, so the check is a guard table and a trigger: the count
-- of rows still holding bytes is written into the table, and any count but
-- zero aborts this migration, naming the remedy, before the table is
-- touched. The table is then rebuilt without the column, keeping every row
-- it holds.
--
-- SQLite is the local development database. One whose server never ran the
-- originals drain (a file-backed server ran it 15 minutes after start, from
-- group E until this migration) refuses here and no code is left to drain
-- it: a database reset (bin/setup-db-with-seeds-and-oauth-and-start-servers.sh)
-- is the remedy, or clearing the bytes by hand, which is an operator decision.

CREATE TEMP TABLE IF NOT EXISTS uploaded_file_bytes_drop_guard (held INTEGER NOT NULL);
CREATE TEMP TRIGGER IF NOT EXISTS uploaded_file_bytes_drop_refusal
    BEFORE INSERT ON uploaded_file_bytes_drop_guard
    WHEN NEW.held <> 0
BEGIN
    SELECT RAISE(ABORT, 'uploaded_activity_files holds uploaded file(s) not drained to the blob store: refusing to drop file_bytes. Reset the local database, or clear the bytes by hand, which is an operator decision (carnet#1309, migration 20261102120000 header).');
END;
INSERT INTO uploaded_file_bytes_drop_guard
    SELECT count(*) FROM uploaded_activity_files WHERE file_bytes IS NOT NULL;
DROP TRIGGER IF EXISTS uploaded_file_bytes_drop_refusal;
DROP TABLE IF EXISTS uploaded_file_bytes_drop_guard;

CREATE TABLE IF NOT EXISTS uploaded_activity_files_next (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    file_sha256 TEXT NOT NULL,
    byte_size INTEGER NOT NULL,
    uploaded_at TEXT NOT NULL,
    origin TEXT NOT NULL DEFAULT 'upload',
    PRIMARY KEY (tenant_id, user_id, file_sha256)
);

INSERT INTO uploaded_activity_files_next
    (tenant_id, user_id, file_sha256, byte_size, uploaded_at, origin)
SELECT tenant_id, user_id, file_sha256, byte_size, uploaded_at, origin
FROM uploaded_activity_files;

DROP TABLE IF EXISTS uploaded_activity_files;

ALTER TABLE uploaded_activity_files_next RENAME TO uploaded_activity_files;

CREATE INDEX IF NOT EXISTS idx_uploaded_activity_files_user_tenant
    ON uploaded_activity_files(user_id, tenant_id);
