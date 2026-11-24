-- ABOUTME: Routines: what makes the platform speak first, for whom, on which trigger, and when it fires next
-- ABOUTME: One table for built-in routines (the check-in) and routines people build with the agent (ADR-036)
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- One engine decides when every unprompted post fires (carnet#1790). A row is
-- a routine:
--
--   kind            what it does: 'check_in' (built in, one per athlete), and
--                   later the coach brief and routines built with the agent
--   scope           whom it speaks about: me | athlete | group | roster;
--                   scope_ref names the athlete or group
--   trigger_kind    'schedule': days_mask (bit 0 Monday .. bit 6 Sunday) at
--                   local_hour of the recipient's own clock
--   prompt          the instruction a routine built with the agent runs;
--                   NULL for a built-in kind, whose instruction is catalogued
--   state           active | paused (a check-in set to 'off' is paused)
--   next_slot_ms    the schedule slot it fires for next
--   next_fire_ms    when it fires next: the slot, or later while deferred
--   deferrals       times the pending firing waited for the recipient to
--                   finish a conversation
--
-- next_* are NULL until the engine schedules the routine, and are reset to
-- NULL whenever its schedule changes, so a wake queued for the old schedule
-- finds nothing to fire.
CREATE TABLE IF NOT EXISTS routines (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    owner_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    scope TEXT NOT NULL DEFAULT 'me',
    scope_ref TEXT,
    trigger_kind TEXT NOT NULL,
    days_mask INTEGER NOT NULL DEFAULT 0,
    local_hour INTEGER NOT NULL DEFAULT 0,
    prompt TEXT,
    state TEXT NOT NULL,
    next_slot_ms INTEGER,
    next_fire_ms INTEGER,
    deferrals INTEGER NOT NULL DEFAULT 0,
    last_fired_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

-- A built-in kind is one routine per owner; routines built with the agent
-- ('custom') may be many.
CREATE UNIQUE INDEX IF NOT EXISTS idx_routines_builtin_per_owner
    ON routines (tenant_id, owner_user_id, kind) WHERE kind <> 'custom';

CREATE INDEX IF NOT EXISTS idx_routines_tenant_state
    ON routines (tenant_id, state);

-- Every athlete who chose a check-in cadence gets their check-in routine:
-- daily every day, three_per_week Monday-Wednesday-Friday, weekly Monday, all
-- at 08:00; 'off' is a paused routine with no days. The engine schedules each
-- one on its next reconcile.
INSERT INTO routines (id, tenant_id, owner_user_id, kind, scope, trigger_kind,
                      days_mask, local_hour, state, created_at_ms, updated_at_ms)
SELECT
    lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' ||
          substr(hex(randomblob(2)),2) || '-' ||
          substr('89ab', abs(random()) % 4 + 1, 1) ||
          substr(hex(randomblob(2)),2) || '-' || hex(randomblob(6))),
    p.tenant_id,
    p.user_id,
    'check_in',
    'me',
    'schedule',
    CASE p.cadence
        WHEN 'daily' THEN 127
        WHEN 'three_per_week' THEN 21
        WHEN 'weekly' THEN 1
        ELSE 0
    END,
    8,
    CASE WHEN p.cadence = 'off' THEN 'paused' ELSE 'active' END,
    p.updated_at_ms,
    p.updated_at_ms
FROM user_check_in_preferences p
-- SQLite needs a WHERE before an upsert clause on INSERT ... SELECT.
WHERE true
ON CONFLICT DO NOTHING;
