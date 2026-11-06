-- ABOUTME: Lets a backfill job row outlive its run with the outcome it ended on, instead of only being deleted
-- ABOUTME: A one-time history import that ended before its history landed reads as incomplete on the provider card

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A backfill row existed only while its job was owed: a run that finished
-- deleted it. The one-time history import a usr/pwd sign-in beside a Garmin
-- grant runs (carnet#1386) also has to say, once it is over, that it ended
-- before the history landed (the session expired mid-copy), so the card can
-- offer to continue instead of reading as if nothing was ever tried. That
-- import keeps its row with outcome = 'incomplete'. NULL is a job still owed,
-- the only kind the resume sweep claims or reaps; a new ask for the pair
-- replaces an ended row.
ALTER TABLE activity_backfill_jobs ADD COLUMN IF NOT EXISTS outcome TEXT;

-- What the job is. NULL is an ordinary backfill; 'history_import' is that
-- one-time import, recorded by the ask that started it and never inferred
-- later from which rows stand beside it: an ordinary job that was owed
-- before the grant existed stays ordinary, and one the import adopts says so.
ALTER TABLE activity_backfill_jobs ADD COLUMN IF NOT EXISTS kind TEXT;
