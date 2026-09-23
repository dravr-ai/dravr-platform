-- ABOUTME: Gives every stored sleep, recovery and body metric a column of its own
-- ABOUTME: HRV, SpO2, body battery, the athlete's note, sleep stages and heart rate were dropped on write

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The health tables mirror the older live models, so metrics the synced
-- records carry had nowhere to go. HRV was written as a string ("58.2ms")
-- into hrv_status and never read back, so readiness and outcome checks saw no
-- HRV at all; stage durations, sleeping heart rate, SpO2, body battery and BMI
-- were dropped outright. Each now has a column, and the HRV already written
-- as a string is carried into hrv_ms.
ALTER TABLE sleep_sessions ADD COLUMN IF NOT EXISTS deep_sleep_seconds INTEGER;
ALTER TABLE sleep_sessions ADD COLUMN IF NOT EXISTS light_sleep_seconds INTEGER;
ALTER TABLE sleep_sessions ADD COLUMN IF NOT EXISTS rem_sleep_seconds INTEGER;
ALTER TABLE sleep_sessions ADD COLUMN IF NOT EXISTS awake_seconds INTEGER;
ALTER TABLE sleep_sessions ADD COLUMN IF NOT EXISTS avg_heart_rate DOUBLE PRECISION;
ALTER TABLE sleep_sessions ADD COLUMN IF NOT EXISTS min_heart_rate INTEGER;

ALTER TABLE recovery_metrics ADD COLUMN IF NOT EXISTS hrv_ms DOUBLE PRECISION;
ALTER TABLE recovery_metrics ADD COLUMN IF NOT EXISTS hrv_rmssd DOUBLE PRECISION;
ALTER TABLE recovery_metrics ADD COLUMN IF NOT EXISTS body_battery INTEGER;
ALTER TABLE recovery_metrics ADD COLUMN IF NOT EXISTS spo2 DOUBLE PRECISION;
ALTER TABLE recovery_metrics ADD COLUMN IF NOT EXISTS athlete_note TEXT;

ALTER TABLE health_snapshots ADD COLUMN IF NOT EXISTS bmi DOUBLE PRECISION;

UPDATE recovery_metrics
SET hrv_ms = CAST(REPLACE(hrv_status, 'ms', '') AS DOUBLE PRECISION)
WHERE hrv_ms IS NULL AND hrv_status ~ '^[0-9]+(\.[0-9]+)?ms$';
