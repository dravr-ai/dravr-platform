-- ABOUTME: archive_erasures — deleted accounts whose imported history barrage still holds, until it is erased (PostgreSQL version)
-- ABOUTME: Written in the account delete's transaction, removed once barrage confirms; a sweep retries the rest

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- An athlete's imported history (carnet#1218) lives in dravr-barrage: its
-- records and import jobs in its own schema or database file, the original
-- recording files in its blob store. None of it is in a table the account
-- delete's transaction reaches, and the blob store is not a database at all.
-- So the delete writes the account's id here in its own transaction, and a
-- sweep erases everything barrage holds for it, in every tenant, removing the
-- row once barrage confirms. The row holds that id and nothing else about the
-- account; the column is not named user_id, since it outlives the user it
-- names by design and cascades from nothing.
CREATE TABLE IF NOT EXISTS archive_erasures (
    erased_account_id TEXT PRIMARY KEY,
    attempts BIGINT NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_archive_erasures_due ON archive_erasures(next_attempt_at);
