-- ABOUTME: feedback_messages carries the Dravr team's replies, their read state, and an athlete's screenshot (PostgreSQL)
-- ABOUTME: Mirrors migrations/20261104200000_feedback_team_replies.sql with a BIGINT read stamp (carnet#1392)
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Every existing row was written by its athlete, which the default records.
--   author                  'athlete' (the thread's owner) or 'team' (a reply
--                           sent from the feedback room in Slack)
--   replier_slack_user_id   the Slack user who sent a team reply, kept for the
--                           team's own record; the athlete sees "Dravr team"
--   read_at_ms              when the athlete opened the thread after a team
--                           reply landed; NULL on a team row is unread
--   image_sha256            a screenshot the athlete attached: the digest of
--   image_media_type        its bytes, kept in the blob store under their
--                           upload key, and the type it is served as
ALTER TABLE feedback_messages
    ADD COLUMN IF NOT EXISTS author TEXT NOT NULL DEFAULT 'athlete' CHECK (author IN ('athlete', 'team'));
ALTER TABLE feedback_messages ADD COLUMN IF NOT EXISTS replier_slack_user_id TEXT;
ALTER TABLE feedback_messages ADD COLUMN IF NOT EXISTS read_at_ms BIGINT;
ALTER TABLE feedback_messages ADD COLUMN IF NOT EXISTS image_sha256 TEXT;
ALTER TABLE feedback_messages ADD COLUMN IF NOT EXISTS image_media_type TEXT;

-- The unread count: one athlete's team replies they have not opened yet.
CREATE INDEX IF NOT EXISTS idx_feedback_messages_unread_team
    ON feedback_messages (tenant_id, user_id)
    WHERE author = 'team' AND read_at_ms IS NULL;
