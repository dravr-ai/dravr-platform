-- ABOUTME: Records on each member link row whether it is a coach's write onto the member, not a read of them (PostgreSQL)
-- ABOUTME: Additive, defaulting FALSE: a written row keeps a coach read's audience, but a replay of it draws on no one

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- carnet#1511 call 35: a coach's own plan or calendar write onto an athlete
-- is never a read of them, in its turn or on a replay of the rows it
-- stamped. Each member link row records whether it is such a write:
--
-- written  TRUE for a coach's write onto the member (prescribe_workout,
--          save_training_plan and the like). via_kind stays 'coach' for
--          such a row (its CHECK is unchanged), so it is read again exactly
--          as a coach's read is; the key a reader decides it by names the
--          way 'written', which a replay never counts as drawn on.
--
-- The default is FALSE: every link row written before this reads as before.
ALTER TABLE chat_message_members ADD COLUMN IF NOT EXISTS written BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE group_transcript_entry_members ADD COLUMN IF NOT EXISTS written BOOLEAN NOT NULL DEFAULT FALSE;
