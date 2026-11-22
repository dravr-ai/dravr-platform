-- ABOUTME: A member's opt-in letting their human coach's AI assistant include them in the roster tools (SQLite)
-- ABOUTME: Adds coaching_group_members.coach_assistant_read (default off) and coach_assistant_read_at (carnet#1511)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- coach_assistant_read: the AI assistant the group's human coach connects
-- to Dravr (an MCP client, an A2A agent, an API key) includes this member in
-- the coach's roster tools: get_roster_overview, get_group_member_activities
-- and get_group_member_plan_status. Off until the member turns it on from
-- Dravr's own app, per membership; revocable from anywhere. It does not
-- govern the coach's reads of the member by name, which coach_sharing_consent
-- governs. coach_assistant_read_at records when the member last turned it on
-- or off, NULL while they never did. Additive; no backfill.
ALTER TABLE coaching_group_members ADD COLUMN coach_assistant_read INTEGER NOT NULL DEFAULT 0; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE coaching_group_members ADD COLUMN coach_assistant_read_at TEXT; -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
