-- ABOUTME: Names every group member a chat row is about when it is about several, so no such row is stored as about nobody (PostgreSQL)
-- ABOUTME: Additive only: one new table; a row about one member keeps naming them in chat_messages.about_user_id

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A tool round or a reply that read several group members (a roster naming
-- Alex and Bea) could name none of them in chat_messages.about_user_id, which
-- holds one, so it was stored as about nobody and replayed into any room the
-- conversation later became (carnet#1422). Each member such a row read is a
-- row here. A conversation's member reads are then every about_user_id and
-- every row of this table: the replay keeps nothing from the first read of a
-- member the replaying turn may not read onward, and the row goes with each
-- member's account (SQLITE_USER_PURGE deletes the message, not only the link).
CREATE TABLE IF NOT EXISTS chat_message_members (
    message_id     VARCHAR(255) NOT NULL REFERENCES chat_messages(id) ON DELETE CASCADE,
    member_user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, member_user_id)
);

-- The account delete finds a member's rows by the member.
CREATE INDEX IF NOT EXISTS idx_chat_message_members_member
    ON chat_message_members(member_user_id);
