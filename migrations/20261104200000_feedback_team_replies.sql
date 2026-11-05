-- ABOUTME: feedback_messages carries the Dravr team's replies from Slack, their read state, and an athlete's screenshot (carnet#1392)
-- ABOUTME: author tells the athlete's rows from the team's; read_at_ms drives the unread dot; image_* names a kept screenshot

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
ALTER TABLE feedback_messages ADD COLUMN author TEXT NOT NULL DEFAULT 'athlete' CHECK (author IN ('athlete', 'team')); -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE feedback_messages ADD COLUMN replier_slack_user_id TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE feedback_messages ADD COLUMN read_at_ms INTEGER; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE feedback_messages ADD COLUMN image_sha256 TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE feedback_messages ADD COLUMN image_media_type TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- The unread count: one athlete's team replies they have not opened yet.
CREATE INDEX IF NOT EXISTS idx_feedback_messages_unread_team
    ON feedback_messages (tenant_id, user_id)
    WHERE author = 'team' AND read_at_ms IS NULL;
