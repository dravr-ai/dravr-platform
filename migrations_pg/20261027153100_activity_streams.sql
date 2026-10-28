-- ABOUTME: activity_streams — the one per-activity sample store: pushed, uploaded or imported samples, one row per activity
-- ABOUTME: Mirrors migrations/20261027153100_activity_streams.sql with a uuid user_id and PG-native timestamps
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Every activity whose recorded samples Dravr holds itself keeps them here:
-- samples a provider pushed instead of serving them on request (the Garmin
-- Health API's activity details), samples decoded from a file the athlete
-- uploaded, and samples from an export archive they imported. `origin` says
-- which. They are kept apart from the cached activity row: every list read
-- deserializes that row, and an hour of 1 Hz samples would multiply what each
-- list read carries; a pushed summary and its samples also arrive in separate
-- pushes, and a later summary rewrites the cached row whole.
--
-- PURGE CONTRACT: a provider purge (disconnect, cache expiry, termination)
-- deletes only this provider's rows with origin = 'push'. Upload and archive
-- rows are the athlete's own record and go only with the athlete or their
-- account (users ON DELETE CASCADE and the account-deletion sweep).
--
-- source_ref names what the samples were decoded from: the SHA-256 of the
-- original file, or a blob id; NULL for samples that arrived as JSON.
-- format_version is the version of streams_json's shape (the TimeSeriesData
-- serialization), so a later shape is never misread as this one.
--
-- user_id is uuid, a foreign key to users like SQLite's; statements bind the
-- hyphenated id and cast it (`$n::uuid`), and the purges compare
-- CAST(user_id AS TEXT). tenant_id is TEXT, like cached_activities.

CREATE TABLE IF NOT EXISTS activity_streams (
    tenant_id TEXT NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    activity_id TEXT NOT NULL,
    origin TEXT NOT NULL CHECK (origin IN ('push', 'upload', 'archive')),
    source_ref TEXT,
    format_version SMALLINT NOT NULL,
    streams_json TEXT NOT NULL,
    synced_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, user_id, provider, activity_id)
);

CREATE INDEX IF NOT EXISTS idx_activity_streams_user_tenant
    ON activity_streams(user_id, tenant_id);

CREATE INDEX IF NOT EXISTS idx_activity_streams_provider_origin_synced
    ON activity_streams(provider, origin, synced_at);
