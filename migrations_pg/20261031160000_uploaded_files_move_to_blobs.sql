-- ABOUTME: Uploaded files move to the blob store: the row keeps the file's metadata, its bytes drain out (Postgres)
-- ABOUTME: Adds origin and lets file_bytes be NULL; the column is dropped once every row is drained (Consolidation S12)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- An uploaded .fit file is kept in the blob store, beside the recordings an
-- archive import keeps, under the key upload/<sha256> of the uploader's
-- tenant and account. This table keeps what names the file: its owner, its
-- hash, its size, its origin and when it came in. A new upload writes no
-- bytes here.
--
-- The bytes already stored here are copied to the blob store by the
-- originals drain worker, each verified against its hash before the row's
-- file_bytes is cleared. A migration cannot write to the blob store, and an
-- older instance still serving during a rollout keeps writing bytes here,
-- so the column is dropped in a later migration, once no row holds bytes.

ALTER TABLE uploaded_activity_files
    ADD COLUMN IF NOT EXISTS origin TEXT NOT NULL DEFAULT 'upload';

ALTER TABLE uploaded_activity_files
    ALTER COLUMN file_bytes DROP NOT NULL;
