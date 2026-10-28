-- ABOUTME: pushed_activity_files — a TRANSIENT retry buffer (<= 24 h) for a pushed activity file, never a file store
-- ABOUTME: Mirrors migrations/20261027210000_pushed_activity_files.sql with BYTEA and PG-native timestamps
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Garmin offers an activity's FIT file once, through a callback URL that
-- answers 410 after its first download. A file held only in memory is lost
-- for good when the write after the download fails or the instance dies, so
-- the bytes are kept here from the download until the samples and laps they
-- carry are stored, and every retry decodes this copy instead of asking
-- Garmin again. Rows name their provider so the provider-disconnect purge
-- deletes them with the rest of that provider's data. Ids are TEXT, like
-- activity_streams, so the purge's statements bind the same way on both
-- engines.

-- NOT A FILE STORE. A row lives only from the download until the samples
-- and laps decoded from it land in activity_streams and beside the cached
-- row (activity_streams.source_ref keeps the file's SHA-256), and the
-- worker deletes any row older than 24 hours whatever became of it. Nothing
-- reads a file from here but the retry that decodes it. Originals the
-- athlete keeps are uploaded_activity_files' (keyed by hash, read as the
-- athlete's own uploads), which is why a pushed file is not kept there.

CREATE TABLE IF NOT EXISTS pushed_activity_files (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    activity_id TEXT NOT NULL,
    file_bytes BYTEA NOT NULL,
    fetched_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, user_id, provider, activity_id)
);

CREATE INDEX IF NOT EXISTS idx_pushed_activity_files_user_tenant
    ON pushed_activity_files(user_id, tenant_id);

CREATE INDEX IF NOT EXISTS idx_pushed_activity_files_fetched_at
    ON pushed_activity_files(fetched_at);
