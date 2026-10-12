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
-- PostgreSQL named the inline column constraint
-- group_transcript_entries_speaker_check; it is replaced under the same name
-- so the table keeps exactly one speaker CHECK.
ALTER TABLE group_transcript_entries
    ADD COLUMN IF NOT EXISTS corrects_entry_id UUID REFERENCES group_transcript_entries(id) ON DELETE CASCADE;
ALTER TABLE group_transcript_entries
    DROP CONSTRAINT IF EXISTS group_transcript_entries_speaker_check;
ALTER TABLE group_transcript_entries
    ADD CONSTRAINT group_transcript_entries_speaker_check
    CHECK (speaker IN ('member', 'coach', 'correction'));
ALTER TABLE group_transcript_entries
    DROP CONSTRAINT IF EXISTS group_transcript_entries_correction_target_check;
ALTER TABLE group_transcript_entries
    ADD CONSTRAINT group_transcript_entries_correction_target_check
    CHECK ((speaker = 'correction') = (corrects_entry_id IS NOT NULL));

-- coach_reply_reviews: one row per flag or correction a group's coach made
-- on one of the agent's replies in that group. tenant_id is the group's
-- tenant — a coach coaches only in their own tenant — and every read filters
-- on it together with the coach. A flag carries its reason and note and is
-- never shown to the athlete; a correction carries the room entry it posted.
CREATE TABLE IF NOT EXISTS coach_reply_reviews (
    id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    group_id UUID NOT NULL REFERENCES coaching_groups(id) ON DELETE CASCADE,
    coach_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    athlete_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    reply_entry_id UUID NOT NULL REFERENCES group_transcript_entries(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('flag', 'correction')),
    reason TEXT CHECK (reason IN ('wrong', 'unsafe', 'off_tone')),
    note TEXT,
    correction_entry_id UUID REFERENCES group_transcript_entries(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL,
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
