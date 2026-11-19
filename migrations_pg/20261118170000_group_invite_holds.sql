-- ABOUTME: group_invite_holds — the group invite an account opened before it could join, held until the account is active (PostgreSQL)
-- ABOUTME: Replaces the tab-local copy of the code that the email-confirmation tab and the post-sign-in reload lost (carnet#1648)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- A group invite link (/groups/join/<code>) opened signed out reaches the
-- server with the sign-up, or right after the sign-in, and waits here until
-- the account is active and joins. One hold per account: a newer link
-- replaces the older one. Like signup_code_redemptions there is no tenant:
-- the hold can precede the account's tenant, and the invite names the
-- group's own. The hold goes with the account and with the invite.
CREATE TABLE IF NOT EXISTS group_invite_holds (
    user_id    UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    invite_id  UUID NOT NULL REFERENCES group_invites(id) ON DELETE CASCADE,
    held_at    TIMESTAMPTZ NOT NULL
);

-- Deleting an invite cascades through its holds.
CREATE INDEX IF NOT EXISTS idx_group_invite_holds_invite
    ON group_invite_holds (invite_id);
