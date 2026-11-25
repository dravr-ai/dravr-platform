-- ABOUTME: plan_change_events — one row per change a coach or the thread's agent made to an athlete's plan or the coach's draft of it (SQLite)
-- ABOUTME: Written beside each day write, agent draft save, publish and discard; read newest first in the coach's thread (carnet#1815)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A coach drafts an athlete's plan with the thread's agent. Every change
-- either of them makes writes one row here, after the change itself landed:
-- a history write that fails is logged and never undoes or refuses the
-- change. The coach's thread lists the coach's own rows, newest first.
--   tenant_id         the athlete's one home tenant, where the plan lives;
--                     every read and write filters on it with the athlete
--   athlete_user_id   whose plan changed: the row's owner
--   coach_user_id     the coach whose draft or publish it was; NULL once that
--                     account is deleted, so the athlete keeps the record
--   source            who made the change: 'coach' (their own day editor or
--                     buttons) or 'agent' (the thread's agent, on the
--                     coach's word)
--   kind              'day_edit' | 'day_add' | 'day_remove' | 'day_restore'
--                     | 'date_revert' | 'coach_call' | 'agent_draft'
--                     | 'publish' | 'discard'
--   dates             JSON array of the 'YYYY-MM-DD' dates the change
--                     touched, ascending; '[]' for a change that names none
--   created_at        when the change landed
-- source and kind carry no CHECK: a later kind or source is a new value the
-- server writes, never a table rebuild, and the server refuses a value it
-- does not know when it reads one back.
-- Additive; no backfill: changes before this table have no record.
CREATE TABLE IF NOT EXISTS plan_change_events (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    athlete_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    coach_user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    source TEXT NOT NULL,
    kind TEXT NOT NULL,
    dates TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL
);

-- The coach's listing: their events on one athlete's plan, newest first.
CREATE INDEX IF NOT EXISTS idx_plan_change_events_listing
    ON plan_change_events (tenant_id, athlete_user_id, coach_user_id, created_at);
