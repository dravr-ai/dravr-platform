-- ABOUTME: Records on each member link row whether a coach-way training read went beyond the member's activities (PostgreSQL)
-- ABOUTME: Additive, defaulting FALSE: a plan or physiology read is then read by its coach alone, never through peer sharing

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- carnet#1511 call 23: a member's peer sharing covers their activities alone.
-- A coach-way read of their plan, a session or draft of it, or their
-- physiology is a training read no peer view covers. Each member link row
-- records whether its read went beyond the member's activities:
--
-- beyond_activities  TRUE for a training read of anything but activities,
--                    read again by the coach who read it alone. data_class
--                    stays 'training' for such a row (its CHECK is
--                    unchanged); the key a reader decides it by names the
--                    class beyond_activities.
--
-- The default is FALSE: every link row written before this is a branch
-- database's (no deployed database has these tables), read as before.
ALTER TABLE chat_message_members ADD COLUMN IF NOT EXISTS beyond_activities BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE group_transcript_entry_members ADD COLUMN IF NOT EXISTS beyond_activities BOOLEAN NOT NULL DEFAULT FALSE;
