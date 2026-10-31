-- ABOUTME: Drops data_point_series_archive, the daily rollup table no code ever wrote (Postgres)
-- ABOUTME: Refuses, failing the boot, if the table holds a single row (Athlete Data Consolidation, S10)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The health persistence schema created data_point_series_archive for daily
-- rollups of the time-series points. No writer was ever built: only the
-- purges deleted from it. A day's wellness lives on the day's rows
-- (recovery_metrics, health_snapshots, daily_activity) and intraday samples
-- in data_point_series, so the table is dropped with its purge statements.
--
-- The drop is one-way, so it checks first: a table holding any row stops
-- this migration, and with it the boot, rather than deleting data nothing
-- has accounted for.

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM data_point_series_archive) THEN
        RAISE EXCEPTION 'data_point_series_archive holds rows: refusing to drop it';
    END IF;
END $$;

DROP INDEX IF EXISTS idx_archive_source_type;
DROP TABLE IF EXISTS data_point_series_archive;
