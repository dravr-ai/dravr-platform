-- ABOUTME: user_export_bundles — the athlete's originals bundles: the async build of their kept files, and its state (SQLite)
-- ABOUTME: One building row per athlete; a ready bundle is downloadable for 7 days, then the sweep deletes its blob and row (carnet#1541)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- GET /api/user/export carries the athlete's rows; the bytes it leaves out
-- (kept originals, uploaded recordings, feedback screenshots) are built on
-- request into one ZIP in the blob store, under
-- t/<tenant>/u/<user>/export_bundle/<id>, and the export names the latest.
--   id            the bundle's id, also its blob key's id
--   tenant_id     the session's tenant; the blob is keyed under it
--   user_id       the athlete, the row's owner
--   status        building, ready or failed
--   sealed        whether the athlete sealed it with a passphrase, which is
--                 never stored
--   files, missing, damaged   what the bundle holds; NULL while building
--   byte_size, sha256         the stored file the athlete downloads
--   error         why a build failed, never a path or a credential
--   created_at    when it was asked for
--   heartbeat_at  moved forward while it builds: a building row whose
--                 heartbeat stopped belongs to an instance that died, and
--                 the sweep fails it
--   finished_at   when it became ready or failed
--   expires_at    when the sweep deletes its blob and the row
-- Additive; no backfill.
CREATE TABLE IF NOT EXISTS user_export_bundles (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN ('building', 'ready', 'failed')),
    sealed INTEGER NOT NULL DEFAULT 0 CHECK (sealed IN (0, 1)),
    files INTEGER,
    missing INTEGER,
    damaged INTEGER,
    byte_size INTEGER,
    sha256 TEXT,
    error TEXT,
    created_at TEXT NOT NULL,
    heartbeat_at TEXT NOT NULL,
    finished_at TEXT,
    expires_at TEXT NOT NULL
);

-- The athlete's latest bundle.
CREATE INDEX IF NOT EXISTS idx_user_export_bundles_user
    ON user_export_bundles (tenant_id, user_id, created_at);

-- The sweep: bundles past their expiry.
CREATE INDEX IF NOT EXISTS idx_user_export_bundles_expiry
    ON user_export_bundles (expires_at);

-- One build at a time per athlete: a second request while one builds is a
-- conflict here, and reads back the one building.
CREATE UNIQUE INDEX IF NOT EXISTS one_building_bundle_per_user
    ON user_export_bundles (tenant_id, user_id) WHERE status = 'building';
