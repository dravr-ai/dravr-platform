-- ABOUTME: Drops uploaded_activity_files.file_bytes: every uploaded file's bytes live in the blob store (Postgres)
-- ABOUTME: Refuses, failing the boot, while any row still holds bytes (Athlete Data Consolidation S12, carnet#1309)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Migration 20261031160000 moved uploaded files to the blob store and let
-- file_bytes be NULL; the originals drain then copied each row's bytes out,
-- verified against the file's hash, and cleared the column. With the drain
-- finished, the row names the file and the blob store alone holds it, so
-- the column goes with the code that read and wrote it.
--
-- The drop is one-way, so it checks first: a row still holding bytes stops
-- this migration, and with it the boot, rather than deleting a file the
-- blob store may not hold. The revision serving before this one keeps
-- serving, and its drain keeps copying files out, so a file it had not yet
-- reached drains by itself and the next deploy goes through.
--
-- A file whose bytes do not match the size or hash its row records never
-- drains: the drain refused it on every pass, and no code is left after this
-- migration to clear it. Those bytes are not the file the row names, so
-- nothing can recover it; clearing them or deleting the row is an operator
-- decision, and the refusal counts such files so the operator can find them:
--   SELECT tenant_id::text, user_id::text, file_sha256 FROM uploaded_activity_files
--   WHERE file_bytes IS NOT NULL AND (length(file_bytes) <> byte_size
--         OR encode(sha256(file_bytes), 'hex') <> file_sha256);

DO $$
DECLARE
    held BIGINT;
    unrecoverable BIGINT;
BEGIN
    SELECT count(*),
           count(*) FILTER (WHERE length(file_bytes) <> byte_size
                               OR encode(sha256(file_bytes), 'hex') <> file_sha256)
      INTO held, unrecoverable
      FROM uploaded_activity_files
     WHERE file_bytes IS NOT NULL;
    IF held > 0 THEN
        RAISE EXCEPTION 'uploaded_activity_files holds % uploaded file(s) not drained to the blob store, % of them not matching their recorded size or hash: refusing to drop file_bytes. The previous revision''s drain copies out the files that match; one that does not can never drain, and clearing its bytes or deleting its row is an operator decision (carnet#1309, migration 20261102120000 header).', held, unrecoverable;
    END IF;
END $$;

ALTER TABLE uploaded_activity_files DROP COLUMN IF EXISTS file_bytes;
