-- ABOUTME: Drops the seven wellness columns nothing writes, after moving any stored night respiration to its morning
-- ABOUTME: Athlete Data Consolidation S13 step 2 (carnet#1280); mirrors migrations/20261101120000

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Since S13 step 1 (02394dff9) no statement names these columns, and since
-- lane G (801a0f6bd) a night's respiratory rate lands in the morning's
-- recovery_metrics.resting_respiratory_rate, where every source files it.
--
-- 1. Copy. A stored sleep_sessions.respiratory_rate goes to a recovery row
--    of the same athlete and tenant whose date is the night's morning: the
--    date its end_time falls on in the athlete's zone (users.timezone, UTC
--    when unset or not a zone PostgreSQL knows, so a bad value cannot fail
--    this migration). Only a day that holds no rate takes one, and no day row
--    is created: the copy is the column the merge already reads, never a new
--    record. A day takes its own provider's night first, then the main night
--    ahead of a nap, then the longer sleep (start time and id settle a tie).
--    Soft-deleted rows on either side are left alone. The nights are read
--    once, zone included, into a materialized set, so the zone catalogue is
--    scanned once rather than per recovery row; a database without any
--    stored rate (dev on 2026-11-01) updates nothing.
WITH nights AS MATERIALIZED (
    SELECT s.id, s.user_id, s.tenant_id, s.provider, s.respiratory_rate,
           s.is_nap, s.total_sleep_time, s.start_time,
           (s.end_time AT TIME ZONE COALESCE(z.name, 'UTC'))::date AS morning
    FROM sleep_sessions s
    LEFT JOIN users u ON u.id::text = s.user_id
    LEFT JOIN pg_timezone_names z ON z.name = u.timezone
    WHERE s.respiratory_rate IS NOT NULL
      AND s.deleted_at IS NULL
)
UPDATE recovery_metrics rm
SET resting_respiratory_rate = (
    SELECT n.respiratory_rate
    FROM nights n
    WHERE n.user_id = rm.user_id
      AND n.tenant_id = rm.tenant_id
      AND n.morning = rm.date
    ORDER BY (n.provider = rm.provider) DESC, n.is_nap,
             n.total_sleep_time DESC NULLS LAST, n.start_time, n.id
    LIMIT 1
)
WHERE rm.resting_respiratory_rate IS NULL
  AND rm.deleted_at IS NULL
  AND EXISTS (
      SELECT 1 FROM nights n
      WHERE n.user_id = rm.user_id
        AND n.tenant_id = rm.tenant_id
        AND n.morning = rm.date
  );

-- 2. Drop. Each value has a home or no source: the night's score is
--    sleep_sessions.sleep_score, the skin temperature is
--    recovery_metrics.body_temperature, Garmin's BMR is
--    daily_activity.total_kcal, HRV is hrv_ms/hrv_rmssd; the onset latency
--    and wake count were carried by no shipped mapping.
ALTER TABLE sleep_sessions DROP COLUMN IF EXISTS respiratory_rate;
ALTER TABLE sleep_sessions DROP COLUMN IF EXISTS temperature_variation;
ALTER TABLE sleep_sessions DROP COLUMN IF EXISTS wake_count;
ALTER TABLE sleep_sessions DROP COLUMN IF EXISTS sleep_onset_latency;
ALTER TABLE recovery_metrics DROP COLUMN IF EXISTS hrv_status;
ALTER TABLE recovery_metrics DROP COLUMN IF EXISTS sleep_score;
ALTER TABLE health_snapshots DROP COLUMN IF EXISTS bmr;
