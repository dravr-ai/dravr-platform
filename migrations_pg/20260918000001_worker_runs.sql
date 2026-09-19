-- ABOUTME: One row per periodic worker recording when it last completed a tick and who holds the current one
-- ABOUTME: Lets a worker resume its schedule across process restarts instead of restarting its countdown at boot
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A worker's first tick used to fire one full period after boot, so on a
-- scale-to-zero service whose instances rarely live an hour, a weekly digest
-- never ran at all (carnet#459). The ledger makes "due" a fact about the
-- worker, not the process: a fresh instance reads last_run_at_ms and waits
-- only the remainder, and an overdue tick fires shortly after boot.
--
-- leased_until_ms is the claim: an instance takes an overdue tick by setting
-- it, so two instances booting together run it once. A tick that dies
-- leaves last_run_at_ms untouched, and the next boot after the lease retries.
CREATE TABLE IF NOT EXISTS worker_runs (
    name TEXT PRIMARY KEY,
    last_run_at_ms BIGINT NOT NULL DEFAULT 0,
    leased_until_ms BIGINT NOT NULL DEFAULT 0
);
