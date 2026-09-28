-- ABOUTME: form_ctl on training_history — the CTL a day's form is a share of (SQLite)
-- ABOUTME: Clears every row stored under same-day form; the rollup recomputes them from the activity cache

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Form now follows the Coggan/TrainingPeaks convention (dravr-cageux 0.25,
-- carnet#601): TSB on a day is CTL minus ATL at the end of the day before,
-- read as a share of that same day's CTL, which the row stores as form_ctl.
-- Every stored row holds the former same-day TSB, which decayed a rest day
-- that had not happened and read a daily rider as fresh at noon, and it has no
-- form_ctl to band against. The rollup is derived data the next capture or
-- compute_training_history call rebuilds from the durable activity cache, so
-- the stale rows are deleted rather than served. The column stays nullable:
-- a row an older binary writes during a rollout carries no form_ctl, and the
-- read query never returns a row without one.
DELETE FROM training_history;
ALTER TABLE training_history ADD COLUMN form_ctl REAL; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
