-- ABOUTME: Binds a human coach's conversation to the one athlete it works on, and the group that lets it (carnet#1070)
-- ABOUTME: Two nullable columns, one partial unique index and a CHECK that a subject thread is never a room nor a personal thread

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A group's human coach works on one athlete's plan in a thread of their own
-- bound to that athlete: subject_user_id names the athlete, subject_group_id
-- the coached group whose consent and membership let the coach act for them.
-- Both are NULL on every other thread, and both are set together.
--
-- A thread about an athlete goes with the athlete's account: it carries their
-- data under the coach's view, which must not outlive them.
ALTER TABLE chat_conversations
    ADD COLUMN IF NOT EXISTS subject_user_id UUID REFERENCES users(id) ON DELETE CASCADE;
ALTER TABLE chat_conversations
    ADD COLUMN IF NOT EXISTS subject_group_id UUID REFERENCES coaching_groups(id);

-- One thread per coach per athlete in a tenant: reopening it from the roster
-- returns this row, and two opens racing each other end on the same one.
CREATE UNIQUE INDEX IF NOT EXISTS idx_chat_conversations_subject
    ON chat_conversations (tenant_id, user_id, subject_user_id) WHERE subject_user_id IS NOT NULL;

-- The thread is the coach's own: never a room, whose replies reach every
-- member, and never the coach's personal thread, which Home opens and the
-- platform posts into. The SQLite repository refuses the same writes.
ALTER TABLE chat_conversations
    DROP CONSTRAINT IF EXISTS chat_conversations_subject_check;
ALTER TABLE chat_conversations
    ADD CONSTRAINT chat_conversations_subject_check
    CHECK (
        (subject_user_id IS NULL AND subject_group_id IS NULL)
        OR (subject_user_id IS NOT NULL AND subject_group_id IS NOT NULL
            AND group_id IS NULL AND personal = FALSE)
    );
