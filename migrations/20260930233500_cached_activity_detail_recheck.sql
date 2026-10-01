-- ABOUTME: cached_activities.detail_recheck_at — when a stored detail read that found neither splits nor laps is read again
-- ABOUTME: NULL for a detail that carried either, which stands; a detail stored before the column existed is read once more
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- A detail read that carried neither splits nor laps proves nothing: a
-- provider can serve its activity without them when the sibling request that
-- carries them fails (Garmin's /laps and /splits), and the answer reads the
-- same as an activity that has none. Such a detail is stored with the instant
-- it is read again; past it, the row counts as not read and the activity view
-- asks the provider once more. A detail that carried either stands, NULL.
--
-- The empty details stored before this column proved nothing either, so they
-- are given an instant already past and read once more.

ALTER TABLE cached_activities ADD COLUMN detail_recheck_at TEXT;  -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

UPDATE cached_activities
SET detail_recheck_at = synced_at
WHERE detail_json = '{}' AND detail_recheck_at IS NULL;
