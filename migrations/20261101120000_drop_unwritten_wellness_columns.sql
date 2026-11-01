-- ABOUTME: Drops the seven wellness columns nothing writes, after moving any stored night respiration to its morning (SQLite)
-- ABOUTME: Athlete Data Consolidation S13 step 2 (carnet#1280); mirrors migrations_pg/20261101120000

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- See the PostgreSQL migration of the same version for why. One difference:
-- SQLite knows no time zones, so the night's morning is the UTC date of its
-- end_time. SQLite holds no deployed athlete's data.

-- SQLite resolves no outer column in a subquery's ORDER BY, so the day's
-- own provider goes first as a pass of its own; the second pass fills a
-- day still empty from any of the athlete's nights that morning.
WITH nights AS (
    SELECT s.id, s.user_id, s.tenant_id, s.provider, s.respiratory_rate,
           s.is_nap, s.total_sleep_time, s.start_time,
           date(s.end_time) AS morning
    FROM sleep_sessions s
    WHERE s.respiratory_rate IS NOT NULL
      AND s.deleted_at IS NULL
)
UPDATE recovery_metrics
SET resting_respiratory_rate = (
    SELECT n.respiratory_rate
    FROM nights n
    WHERE n.user_id = recovery_metrics.user_id
      AND n.tenant_id = recovery_metrics.tenant_id
      AND n.morning = recovery_metrics.date
      AND n.provider = recovery_metrics.provider
    ORDER BY n.is_nap, n.total_sleep_time DESC, n.start_time, n.id
    LIMIT 1
)
WHERE resting_respiratory_rate IS NULL
  AND deleted_at IS NULL
  AND EXISTS (
      SELECT 1 FROM nights n
      WHERE n.user_id = recovery_metrics.user_id
        AND n.tenant_id = recovery_metrics.tenant_id
        AND n.morning = recovery_metrics.date
        AND n.provider = recovery_metrics.provider
  );

WITH nights AS (
    SELECT s.id, s.user_id, s.tenant_id, s.provider, s.respiratory_rate,
           s.is_nap, s.total_sleep_time, s.start_time,
           date(s.end_time) AS morning
    FROM sleep_sessions s
    WHERE s.respiratory_rate IS NOT NULL
      AND s.deleted_at IS NULL
)
UPDATE recovery_metrics
SET resting_respiratory_rate = (
    SELECT n.respiratory_rate
    FROM nights n
    WHERE n.user_id = recovery_metrics.user_id
      AND n.tenant_id = recovery_metrics.tenant_id
      AND n.morning = recovery_metrics.date
    ORDER BY n.is_nap, n.total_sleep_time DESC, n.start_time, n.id
    LIMIT 1
)
WHERE resting_respiratory_rate IS NULL
  AND deleted_at IS NULL
  AND EXISTS (
      SELECT 1 FROM nights n
      WHERE n.user_id = recovery_metrics.user_id
        AND n.tenant_id = recovery_metrics.tenant_id
        AND n.morning = recovery_metrics.date
  );

ALTER TABLE sleep_sessions DROP COLUMN respiratory_rate; -- idempotency-ok: SQLite DROP COLUMN has no IF EXISTS; _sqlx_migrations prevents a re-run
ALTER TABLE sleep_sessions DROP COLUMN temperature_variation; -- idempotency-ok: SQLite DROP COLUMN has no IF EXISTS; _sqlx_migrations prevents a re-run
ALTER TABLE sleep_sessions DROP COLUMN wake_count; -- idempotency-ok: SQLite DROP COLUMN has no IF EXISTS; _sqlx_migrations prevents a re-run
ALTER TABLE sleep_sessions DROP COLUMN sleep_onset_latency; -- idempotency-ok: SQLite DROP COLUMN has no IF EXISTS; _sqlx_migrations prevents a re-run
ALTER TABLE recovery_metrics DROP COLUMN hrv_status; -- idempotency-ok: SQLite DROP COLUMN has no IF EXISTS; _sqlx_migrations prevents a re-run
ALTER TABLE recovery_metrics DROP COLUMN sleep_score; -- idempotency-ok: SQLite DROP COLUMN has no IF EXISTS; _sqlx_migrations prevents a re-run
ALTER TABLE health_snapshots DROP COLUMN bmr; -- idempotency-ok: SQLite DROP COLUMN has no IF EXISTS; _sqlx_migrations prevents a re-run
