-- ABOUTME: deletion_receipts — a hash-chained receipt per account deletion, written in the deletion's own transaction
-- ABOUTME: Names the subject only by a hash of its id; a verifier recomputes every link and reports the first broken one

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- An account delete leaves nothing that names the account, by policy: its
-- provider_data_purges rows go with it. That left no record that a deletion
-- happened at all, nor of what it proved. A receipt is that record, and holds
-- no user id: `subject_hash` is the SHA-256 of the account id, which the
-- athlete (or an operator holding the id) can recompute, and nobody can
-- reverse.
--
-- Each row chains to the one before it: `hash` is the SHA-256 of `prev_hash`
-- followed by the row's other fields in a fixed order, the genesis
-- `prev_hash` being 64 zeros. Every field the hash covers is stored as TEXT
-- exactly as it was hashed (`deleted_at` included, as RFC 3339 with
-- microseconds), so a verifier recomputes it from the stored columns without
-- re-encoding anything. `seq` is unique: two deletions that read the same
-- head collide here, and the later one rolls back whole.
CREATE TABLE IF NOT EXISTS deletion_receipts (
    id TEXT PRIMARY KEY,
    seq INTEGER NOT NULL UNIQUE,
    prev_hash TEXT NOT NULL,
    hash TEXT NOT NULL UNIQUE,
    deleted_at TEXT NOT NULL,
    subject_hash TEXT NOT NULL,
    actor TEXT NOT NULL CHECK (actor IN ('athlete', 'operator', 'messaging')),
    scan_result TEXT NOT NULL,
    revocations TEXT NOT NULL,
    not_deleted TEXT NOT NULL,
    tombstone_ids TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_deletion_receipts_subject ON deletion_receipts(subject_hash);
