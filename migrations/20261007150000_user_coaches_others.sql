-- ABOUTME: users.coaches_others — the onboarding role answer ("I coach others"), kept apart from the style persona
-- ABOUTME: Drives the coach group step and coach-facing agents; coaching_persona keeps choosing only the reply style

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- carnet#827. Until now the role answer was written into coaching_persona as
-- 'coach', which also picked the strict Coach reply contract for the coach's
-- own training talk. The role moves here.
ALTER TABLE users ADD COLUMN coaches_others INTEGER NOT NULL DEFAULT 0;  -- idempotency-ok: SQLite lacks ADD COLUMN IF NOT EXISTS; column is new here

-- Everyone who answered as a coach before the split held the role through the
-- 'coach' persona, so they keep it.
UPDATE users SET coaches_others = 1 WHERE coaching_persona = 'coach';

-- That 'coach' style was set by the role answer, not chosen, so it goes back
-- to the default casual voice (decided 2026-10-07). The Coach style stays
-- selectable in Settings for anyone who wants it.
UPDATE users SET coaching_persona = 'casual' WHERE coaching_persona = 'coach';
