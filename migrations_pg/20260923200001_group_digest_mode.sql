-- ABOUTME: Add digest_mode to coaching_groups — where the group's weekly digest goes, if anywhere
-- ABOUTME: 'off' sends nothing (every existing group), 'chat' posts into the bound chat, 'managers' reaches only the owner and admins
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The weekly digest went out for every group on a tier that includes it, with
-- no way to stop it: the room post is a chat message, which no notification
-- preference reaches (carnet#541). The default is off, and the column default
-- is what every row that exists when this runs takes, so no group sends a
-- digest until its owner, an admin or its attached coach asks for one.
ALTER TABLE coaching_groups
    ADD COLUMN IF NOT EXISTS digest_mode TEXT NOT NULL DEFAULT 'off'
    CHECK (digest_mode IN ('off', 'chat', 'managers'));
