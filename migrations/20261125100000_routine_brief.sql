-- ABOUTME: The coach morning brief's settings on its routine, and every routine firing's run with what it listed
-- ABOUTME: Runs answer "ran 07:00, nothing to report"; listed items let a later Handled or open count as acted on
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Routines P2, the coach morning brief (carnet#1791).
--
--   routines.settings     JSON: what a morning brief leaves out, the attention
--                         reasons it does not check ("reasons_off") and the
--                         athletes it does not list ("muted"); NULL for none
--
--   routine_runs          one row per firing that reached an outcome: posted,
--                         withheld, empty (nothing to report), held, skipped;
--                         items is how many athletes a brief listed, and
--                         not_sharing / unreadable how many it left out
--                         because they don't share or their data could not
--                         be read, so an empty morning still names them
--
--   routine_run_items     the athletes a brief listed, with the attention
--                         reason it listed each for. acted_at_ms and action
--                         (treated | opened) are set once, the first time the
--                         coach marks that athlete Handled or opens them within
--                         24 hours of the brief
ALTER TABLE routines ADD COLUMN settings TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

CREATE TABLE IF NOT EXISTS routine_runs (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    routine_id TEXT NOT NULL REFERENCES routines(id) ON DELETE CASCADE,
    owner_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    slot_ms INTEGER NOT NULL,
    ran_at_ms INTEGER NOT NULL,
    outcome TEXT NOT NULL,
    items INTEGER NOT NULL DEFAULT 0,
    not_sharing INTEGER NOT NULL DEFAULT 0,
    unreadable INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_routine_runs_routine
    ON routine_runs (tenant_id, routine_id, ran_at_ms);

CREATE TABLE IF NOT EXISTS routine_run_items (
    run_id TEXT NOT NULL REFERENCES routine_runs(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    owner_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    athlete_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    reason TEXT NOT NULL,
    listed_at_ms INTEGER NOT NULL,
    acted_at_ms INTEGER,
    action TEXT,
    PRIMARY KEY (run_id, athlete_user_id)
);

CREATE INDEX IF NOT EXISTS idx_routine_run_items_owner
    ON routine_run_items (tenant_id, owner_user_id, athlete_user_id, listed_at_ms);
