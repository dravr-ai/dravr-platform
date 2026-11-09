-- ABOUTME: Removes the catalogue rows of the retired agent_note_add and agent_followup_schedule tools
-- ABOUTME: The agent_notes and agent_followups tables and their rows are untouched
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- carnet#1434 and carnet#1435 retired both tools from the registry. A catalogue
-- row naming an unregistered tool is what run_tool_catalog_sync deletes on the
-- next boot; this says so on a fresh database as well. Any tenant or user
-- override for the two tools cascades with its row, and overrides a tool that no
-- longer exists have nothing left to govern.

DELETE FROM tool_catalog WHERE tool_name IN ('agent_note_add', 'agent_followup_schedule');
