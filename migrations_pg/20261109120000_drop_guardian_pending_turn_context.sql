-- ABOUTME: Drops guardian_pending_actions.turn_agent_id, seat and turn_provenance: no parked call re-dispatches any more (PostgreSQL version)
-- ABOUTME: The table keeps its name and holds only /agent create drafts, which never read these columns

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The three columns let a tool call parked for /confirm re-dispatch as the turn
-- that parked it (20261104120000). carnet#1439 removed parking a tool call: the
-- table holds only the agent drafts /agent create proposes, and no binary since
-- writes or reads these columns (carnet#1471).
ALTER TABLE guardian_pending_actions DROP COLUMN IF EXISTS turn_agent_id;
ALTER TABLE guardian_pending_actions DROP COLUMN IF EXISTS seat;
ALTER TABLE guardian_pending_actions DROP COLUMN IF EXISTS turn_provenance;
