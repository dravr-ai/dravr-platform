-- ABOUTME: Critical power, W′, critical speed and D′ on the physiological profile, each with its provenance
-- ABOUTME: Additive only: four values plus kind (measured | estimated), origin and as-of date per value (carnet#714)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Each value lands with the provenance that decides how it may be framed: a
-- measured value is stated, an estimated one is quoted as an estimate and
-- attributed to its origin. A value is never written without its kind
-- (set_physiology rejects it, and the repository refuses to decode one).

-- Critical power, watts.
ALTER TABLE user_physiological_profiles ADD COLUMN critical_power_watts INTEGER; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE user_physiological_profiles ADD COLUMN critical_power_watts_kind TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE user_physiological_profiles ADD COLUMN critical_power_watts_origin TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE user_physiological_profiles ADD COLUMN critical_power_watts_as_of TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- W′, joules of work capacity above critical power.
ALTER TABLE user_physiological_profiles ADD COLUMN w_prime_joules INTEGER; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE user_physiological_profiles ADD COLUMN w_prime_joules_kind TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE user_physiological_profiles ADD COLUMN w_prime_joules_origin TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE user_physiological_profiles ADD COLUMN w_prime_joules_as_of TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- Critical speed, metres per second.
ALTER TABLE user_physiological_profiles ADD COLUMN critical_speed_mps REAL; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE user_physiological_profiles ADD COLUMN critical_speed_mps_kind TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE user_physiological_profiles ADD COLUMN critical_speed_mps_origin TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE user_physiological_profiles ADD COLUMN critical_speed_mps_as_of TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- D′, metres of distance capacity above critical speed.
ALTER TABLE user_physiological_profiles ADD COLUMN d_prime_meters REAL; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE user_physiological_profiles ADD COLUMN d_prime_meters_kind TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE user_physiological_profiles ADD COLUMN d_prime_meters_origin TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE user_physiological_profiles ADD COLUMN d_prime_meters_as_of TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
