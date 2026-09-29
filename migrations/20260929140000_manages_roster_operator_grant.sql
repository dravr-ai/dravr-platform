-- ABOUTME: Records who granted manages_roster as an operator, and when, so the TrainingPeaks reconciler never takes it back
-- ABOUTME: Written by the admin roster route and pierre-cli user set; read by the reconciler's revoke and the admin views
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- When an operator granted manages_roster; NULL when no operator grant holds.
-- The TrainingPeaks coach connection that earns the grant on its own revokes
-- only a grant this is NULL for.
ALTER TABLE users ADD COLUMN manages_roster_granted_at TEXT;  -- idempotency-ok: SQLite lacks ADD COLUMN IF NOT EXISTS; column is new here
-- The operator who granted it; NULL for a service token, which names no one.
-- No foreign key, as for approved_by: SQLite cannot add one to an existing table.
ALTER TABLE users ADD COLUMN manages_roster_granted_by TEXT;  -- idempotency-ok: SQLite lacks ADD COLUMN IF NOT EXISTS; column is new here
