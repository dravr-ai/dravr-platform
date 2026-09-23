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
ALTER TABLE sleep_sessions ADD COLUMN deep_sleep_seconds INTEGER; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE sleep_sessions ADD COLUMN light_sleep_seconds INTEGER; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE sleep_sessions ADD COLUMN rem_sleep_seconds INTEGER; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE sleep_sessions ADD COLUMN awake_seconds INTEGER; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE sleep_sessions ADD COLUMN avg_heart_rate REAL; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE sleep_sessions ADD COLUMN min_heart_rate INTEGER; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

ALTER TABLE recovery_metrics ADD COLUMN hrv_ms REAL; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE recovery_metrics ADD COLUMN hrv_rmssd REAL; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE recovery_metrics ADD COLUMN body_battery INTEGER; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE recovery_metrics ADD COLUMN spo2 REAL; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE recovery_metrics ADD COLUMN athlete_note TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

ALTER TABLE health_snapshots ADD COLUMN bmi REAL; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

UPDATE recovery_metrics
SET hrv_ms = CAST(REPLACE(hrv_status, 'ms', '') AS REAL)
WHERE hrv_ms IS NULL AND hrv_status LIKE '%ms';
