-- ABOUTME: Drops data_point_series_archive, the daily rollup table no code ever wrote (SQLite)
-- ABOUTME: Refuses if the table holds a single row; mirrors migrations_pg/20261031150000

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the Postgres migration of the same version for why. SQLite has no
-- procedural block, so the check is a CHECK constraint: the row count is
-- written into a table that only accepts zero, and any other count fails
-- this migration before the drop.

CREATE TEMP TABLE IF NOT EXISTS data_point_series_archive_drop_guard (
    held INTEGER NOT NULL CHECK (held = 0)
);
INSERT INTO data_point_series_archive_drop_guard
    SELECT count(*) FROM data_point_series_archive;
DROP TABLE IF EXISTS data_point_series_archive_drop_guard;

DROP INDEX IF EXISTS idx_archive_source_type;
DROP TABLE IF EXISTS data_point_series_archive;
