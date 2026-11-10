-- ABOUTME: Drops guardian_pending_actions.turn_agent_id, seat and turn_provenance: no parked call re-dispatches any more
-- ABOUTME: The table keeps its name and holds only /agent create drafts, which never read these columns

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The three columns let a tool call parked for /confirm re-dispatch as the turn
-- that parked it (20261104120000). carnet#1439 removed parking a tool call: the
-- table holds only the agent drafts /agent create proposes, and no binary since
-- writes or reads these columns (carnet#1471).
ALTER TABLE guardian_pending_actions DROP COLUMN turn_agent_id; -- idempotency-ok: SQLite DROP COLUMN has no IF EXISTS; _sqlx_migrations prevents a re-run
ALTER TABLE guardian_pending_actions DROP COLUMN seat; -- idempotency-ok: SQLite DROP COLUMN has no IF EXISTS; _sqlx_migrations prevents a re-run
ALTER TABLE guardian_pending_actions DROP COLUMN turn_provenance; -- idempotency-ok: SQLite DROP COLUMN has no IF EXISTS; _sqlx_migrations prevents a re-run
