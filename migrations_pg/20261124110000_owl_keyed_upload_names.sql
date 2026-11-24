-- ABOUTME: Keyed upload names and ids (carnet#1701): blob_keyed on the three tables naming an upload, record_base, a lease per file
-- ABOUTME: Expand only: nullable or defaulted columns and partial indexes; the content names stay until carnet#1550 (Postgres)
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- blob_keyed   TRUE once the file is kept under its keyed name
--              (upload/k<hex64>, dravr-barrage BlobKey::keyed_upload). Its
--              content name (upload/<sha256>) may still hold a copy: it is
--              held until carnet#1550 and deleted with the row.
-- record_base  the keyed base of an uploaded file's activity ids
--              (upload-k<hex16>); NULL for a file uploaded before, whose
--              ids carry its digest.

ALTER TABLE uploaded_activity_files ADD COLUMN IF NOT EXISTS blob_keyed BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE uploaded_activity_files ADD COLUMN IF NOT EXISTS record_base TEXT;
ALTER TABLE feedback_messages ADD COLUMN IF NOT EXISTS blob_keyed BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE lab_documents ADD COLUMN IF NOT EXISTS blob_keyed BOOLEAN NOT NULL DEFAULT FALSE;

-- A keyed activity id resolves to its file.
CREATE INDEX IF NOT EXISTS idx_uploaded_activity_files_record_base
    ON uploaded_activity_files (tenant_id, user_id, record_base)
    WHERE record_base IS NOT NULL;

-- The keyed-name backfill pages the files not renamed yet.
CREATE INDEX IF NOT EXISTS idx_uploaded_activity_files_content_named
    ON uploaded_activity_files (tenant_id, user_id, file_sha256)
    WHERE blob_keyed = FALSE;
CREATE INDEX IF NOT EXISTS idx_feedback_messages_content_named
    ON feedback_messages (tenant_id, user_id, image_sha256)
    WHERE blob_keyed = FALSE AND image_sha256 IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_lab_documents_content_named
    ON lab_documents (tenant_id, user_id, file_sha256)
    WHERE blob_keyed = FALSE;

-- One lease per athlete and file: the write, the delete and the keyed-name
-- copy of a file each claim it, in one short statement, and hold no
-- connection while they work. lease_token fences the holder's later writes
-- and its release; a holder that died leaves a lease that lapses at
-- leased_until_ms (epoch milliseconds), and the next claim takes it over.
CREATE TABLE IF NOT EXISTS upload_name_leases (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    file_sha256 TEXT NOT NULL,
    lease_token TEXT NOT NULL,
    leased_until_ms BIGINT NOT NULL,
    PRIMARY KEY (tenant_id, user_id, file_sha256)
);
