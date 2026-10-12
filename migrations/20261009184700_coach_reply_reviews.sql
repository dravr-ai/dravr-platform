-- ABOUTME: A group's coach reviews the agent's replies after they went out — a correction joins the room, a flag stays with the coach
-- ABOUTME: Widens the transcript speaker to 'correction' with the entry it corrects, and records every flag and correction per coach

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- carnet#847. Group replies go straight to the athletes (no review before
-- send), so the coach's control comes after the fact: they flag a reply
-- (wrong, unsafe or off-tone, with a note) and post a correction into the
-- group thread, linked to the reply it corrects.
--
-- A correction is a room utterance: a group_transcript_entries row with
-- speaker 'correction', attributed like the reply it corrects to the member
-- that reply answered, so the room's one consent rule shows it to exactly
-- the readers who could read the reply. corrects_entry_id names that reply.
--
-- SQLite has no ALTER for a CHECK constraint: the table is copied under the
-- widened constraint, the original dropped, the copy renamed and the index
-- recreated. sqlx runs the migration in one transaction and no other table
-- references this one by foreign key until coach_reply_reviews is created
-- below, so no PRAGMA is needed.
CREATE TABLE IF NOT EXISTS group_transcript_entries_rebuild (
    id TEXT PRIMARY KEY,
    group_id TEXT NOT NULL REFERENCES coaching_groups(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    author_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    speaker TEXT NOT NULL CHECK (speaker IN ('member', 'coach', 'correction')),
    content TEXT NOT NULL,
    source_conversation_id TEXT,
    source_message_id TEXT,
    created_at TEXT NOT NULL,
    first_party_only INTEGER NOT NULL DEFAULT 0,
    corrects_entry_id TEXT REFERENCES group_transcript_entries(id) ON DELETE CASCADE,
    CHECK ((speaker = 'correction') = (corrects_entry_id IS NOT NULL))
);
INSERT INTO group_transcript_entries_rebuild (id, group_id, tenant_id, author_user_id, speaker,
    content, source_conversation_id, source_message_id, created_at, first_party_only)
  SELECT id, group_id, tenant_id, author_user_id, speaker, content, source_conversation_id,
    source_message_id, created_at, first_party_only
  FROM group_transcript_entries;
DROP TABLE IF EXISTS group_transcript_entries;
ALTER TABLE group_transcript_entries_rebuild RENAME TO group_transcript_entries;

CREATE INDEX IF NOT EXISTS idx_group_transcript_group_created
    ON group_transcript_entries(group_id, created_at);

-- coach_reply_reviews: one row per flag or correction a group's coach made
-- on one of the agent's replies in that group. tenant_id is the group's
-- tenant — a coach coaches only in their own tenant — and every read filters
-- on it together with the coach. A flag carries its reason and note and is
-- never shown to the athlete; a correction carries the room entry it posted.
CREATE TABLE IF NOT EXISTS coach_reply_reviews (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    group_id TEXT NOT NULL REFERENCES coaching_groups(id) ON DELETE CASCADE,
    coach_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    athlete_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    reply_entry_id TEXT NOT NULL REFERENCES group_transcript_entries(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('flag', 'correction')),
    reason TEXT CHECK (reason IN ('wrong', 'unsafe', 'off_tone')),
    note TEXT,
    correction_entry_id TEXT REFERENCES group_transcript_entries(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    CHECK (
        (kind = 'flag' AND reason IS NOT NULL AND correction_entry_id IS NULL)
        OR (kind = 'correction' AND reason IS NULL AND note IS NULL AND correction_entry_id IS NOT NULL)
    )
);

-- The weekly count and the per-page read both start from the coach.
CREATE INDEX IF NOT EXISTS idx_coach_reply_reviews_coach_created
    ON coach_reply_reviews(tenant_id, coach_user_id, created_at);

-- A reply's own reviews.
CREATE INDEX IF NOT EXISTS idx_coach_reply_reviews_reply
    ON coach_reply_reviews(tenant_id, reply_entry_id);
