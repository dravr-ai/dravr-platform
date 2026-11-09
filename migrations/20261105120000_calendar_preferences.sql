-- ABOUTME: calendar_preferences — the calendar an athlete chose for Dravr's plan and workout pushes (SQLite)
-- ABOUTME: One row per (tenant, athlete); the push target reads it before the ledger and the ranking (carnet#1393)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- An athlete who holds several calendar-writing connections (Garmin,
-- Intervals.icu, Wahoo) names the one Dravr writes to, in the app or in
-- chat, and every later push without a named provider goes there. tenant_id
-- is the tenant the athlete's connections and push ledger live under, and
-- every read and write filters on it with the athlete. `provider` is the
-- backend that writes the calendar, as the push ledger files entries under
-- it. `set_by` is whoever recorded the choice: the athlete, or a group's
-- human coach writing their calendar; NULL once that account is deleted.
-- Additive; no backfill: an athlete with no row has chosen nothing.
CREATE TABLE IF NOT EXISTS calendar_preferences (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    set_by TEXT REFERENCES users(id) ON DELETE SET NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (tenant_id, user_id)
);
