-- ABOUTME: Records whether an outside client's tool call answered with a result, an error, or a refusal (SQLite)
-- ABOUTME: Adds nullable llm_usage outcome ('ok' | 'error' | 'refused'): admin counts include failures, the owner feed lists only 'ok' (carnet#1543)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A direct tool call (call_type 'mcp_tool') by a client outside Dravr's own
-- surfaces writes one llm_usage row however it was answered, as /mcp did
-- before the row moved to the dispatch chokepoint, so admin usage counts
-- include failed calls:
--   outcome  'ok' when the tool's body answered with a result; 'error' when
--            it ran and answered with an error (an isError answer, or a
--            failure it raised); 'refused' when the call was answered before
--            the body ran (a scope, role, Guardian or read-subject refusal,
--            or a failure on the way), whose subject is the caller. The
--            athlete's "who read my data" feed lists only 'ok': any other
--            call served nothing.
-- The row's subject_user_id (20261122120000) also names a group member read
-- through a groups tool: a call with no resolved subject whose body read
-- exactly one member (a coach's get_group_member_activities) names that
-- member; it stays NULL when the body read several. A peer's in-group
-- sharing never reaches an outside client (carnet#1511), so no such row is a
-- peer's read.
-- Additive and nullable; no backfill: rows written before this column, and
-- every row that is not a direct tool call, keep NULL (not guessed).
ALTER TABLE llm_usage ADD COLUMN outcome TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
