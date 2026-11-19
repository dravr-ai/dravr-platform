-- ABOUTME: Records when a member answered the one-time notice of their coach's AI tools (PostgreSQL)
-- ABOUTME: Adds the nullable coaching_group_members.assistant_notice_answered_at (EFVP decision 9.2b)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- When the member answered, in Dravr's own app, the notice that the group's
-- human coach's AI tools read them by name: "Keep sharing" or "Stop
-- sharing". NULL until they do, so every member sharing with a human coach
-- is told once, whenever they joined. A coach-sharing change from an API key,
-- an MCP client or a messaging command never sets it. Additive and nullable;
-- no backfill.
ALTER TABLE coaching_group_members ADD COLUMN IF NOT EXISTS assistant_notice_answered_at TIMESTAMPTZ;
