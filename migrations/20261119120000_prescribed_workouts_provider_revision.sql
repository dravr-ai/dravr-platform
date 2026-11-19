-- ABOUTME: provider_revision on prescribed_workouts: the provider's own change stamp as Dravr's write left the entry (SQLite)
-- ABOUTME: A plan push compares it with the stamp a detail read returns, so an edit made on the provider is not overwritten
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- Garmin's listing carries no change stamp and its workout's `updatedDate`
-- names no time zone, so it cannot be compared with the ledger row's
-- updated_at: a push overwrote an edit the athlete made in Garmin Connect
-- (carnet#1389). Garmin's stamp is now kept exactly as Garmin sent it on
-- the row of the write that produced it, and compared only with Garmin's own
-- stamp on a later read: a different stamp is a change Dravr did not make.
--
-- Opaque TEXT, never parsed as a time. NULL for a provider that sends no such
-- stamp and for every row written before this column; those rows keep the
-- comparison they had.

ALTER TABLE prescribed_workouts ADD COLUMN provider_revision TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
