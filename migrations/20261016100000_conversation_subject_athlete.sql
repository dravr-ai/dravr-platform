-- ABOUTME: Binds a human coach's conversation to the one athlete it works on, and the group that lets it (carnet#1070)
-- ABOUTME: Additive only: two nullable columns and one partial unique index; the subject invariants are enforced by the repository here

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A group's human coach works on one athlete's plan in a thread of their own
-- bound to that athlete: subject_user_id names the athlete, subject_group_id
-- the coached group whose consent and membership let the coach act for them.
-- Both are NULL on every other thread, and both are set together.
--
-- The thread is the coach's own: it is never a room (group_id stays NULL) and
-- never the coach's personal thread. PostgreSQL holds that in a CHECK; adding
-- one here would mean rebuilding chat_conversations, so the repository is what
-- refuses a write that would break it (a subject row is never bound to a group
-- nor designated personal).
--
-- A thread about an athlete goes with the athlete's account: it carries their
-- data under the coach's view, which must not outlive them.
ALTER TABLE chat_conversations ADD COLUMN subject_user_id TEXT REFERENCES users(id) ON DELETE CASCADE; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE chat_conversations ADD COLUMN subject_group_id TEXT REFERENCES coaching_groups(id); -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- One thread per coach per athlete in a tenant: reopening it from the roster
-- returns this row, and two opens racing each other end on the same one.
CREATE UNIQUE INDEX IF NOT EXISTS idx_chat_conversations_subject
    ON chat_conversations (tenant_id, user_id, subject_user_id) WHERE subject_user_id IS NOT NULL;
