-- ABOUTME: Records a room transcript entry's audience when it is written: the coach seat then and every member it drew on (PostgreSQL)
-- ABOUTME: Schema only, no backfill: an entry written before carries no record and is read by its author alone

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A room entry's readers were decided from the group's CURRENT coach and from
-- whether its source chat row still existed: the agent's reply to the coach
-- reached the whole room once the coach was replaced, and a member deleting
-- their thread hid their room words from everyone (carnet#1511 design call
-- 11, round 11). Each entry now records, in the write that makes it:
--
-- coach_at_write     who held the group's coach seat then. The author held
--                    it when author_user_id = coach_at_write: the coach
--                    speaks to the room. Only that coach reads through the
--                    coach's view, and only while still seated.
-- audience_recorded  TRUE when the entry was written with its record. An
--                    entry from before has none, and no record can be
--                    rebuilt for it, so only its author reads it.
--
-- group_transcript_entry_members holds every member other than the author
-- whose data the entry drew on, copied from the source chat row's stamps when
-- it is written. No foreign key on the member: a deleted account's link
-- would otherwise vanish and the entry read as carrying fewer members; the
-- account deletion deletes the entries that name the member instead.
ALTER TABLE group_transcript_entries
    ADD COLUMN IF NOT EXISTS coach_at_write UUID;
ALTER TABLE group_transcript_entries
    ADD COLUMN IF NOT EXISTS audience_recorded BOOLEAN NOT NULL DEFAULT FALSE;

CREATE TABLE IF NOT EXISTS group_transcript_entry_members (
    entry_id       UUID NOT NULL REFERENCES group_transcript_entries(id) ON DELETE CASCADE,
    member_user_id UUID NOT NULL,
    PRIMARY KEY (entry_id, member_user_id)
);

-- The account deletion finds a member's entries by the member.
CREATE INDEX IF NOT EXISTS idx_group_transcript_entry_members_member
    ON group_transcript_entry_members(member_user_id);
