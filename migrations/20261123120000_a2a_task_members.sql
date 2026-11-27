-- ABOUTME: Records whose data an A2A task served, so a data export decides its rows by the per-reader rule (SQLite)
-- ABOUTME: A JSON array of member ids, written once the task ran; NULL marks a task from before it, left out of exports (carnet#1511)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- about_members  the group members whose data the task's tool call read, the
--                caller aside, as a JSON array of user ids: '[]' when it read
--                none. Written by the A2A server once a tool task ran, from
--                the dispatch's own record of the members it read. A task
--                carries no conversation, so this is the only record of whose
--                data it holds. A coaching text turn's task is not stamped:
--                its contextId is the conversation the turn ran in, whose
--                rows decide it. NULL on any other task written before this
--                column, or one that never ran its tool: the export leaves
--                those out, since what they hold cannot be decided.
-- Additive and nullable; no backfill: an old task's reads were never recorded.
ALTER TABLE a2a_tasks ADD COLUMN about_members TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
