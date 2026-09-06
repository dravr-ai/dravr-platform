-- ABOUTME: Agent vocabulary for the tool_catalog display text an operator reads
-- ABOUTME: display_name and description only — tool_name and category are keys and never move
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- ADR-026: the persona an athlete talks to is an agent; a coach is a human.
-- The catalogue rows seeded by 20260407000001 and 20260830000003 still spell
-- the AI persona "coach" in the two columns the admin tool picker renders.
-- This rewrites those two columns, row by row, keyed on tool_name.
--
-- What is deliberately NOT touched:
--   * `tool_name` — the MCP wire name, the FK every tenant_tool_overrides and
--     user_tool_overrides row points at, and the key each UPDATE below reads.
--   * `category` — 'coaches' is one of the values the CHECK constraint admits.
--   * tc-100 `list_coaching_playbooks` — "coaching playbook" is the activity,
--     not the persona, and keeps its name in every locale.
--
-- Idempotent: each statement writes a constant, keyed on a unique tool_name,
-- so a second run computes the same row. A tool_name that is absent updates
-- nothing.

-- Athlete-facing agent tools (category 'coaches').
UPDATE tool_catalog SET
    display_name = 'List Agents',
    description = 'List available AI agents for personalized training guidance'
WHERE tool_name = 'list_coaches';

UPDATE tool_catalog SET
    display_name = 'Create Agent',
    description = 'Create a custom AI agent with personalized training guidance'
WHERE tool_name = 'create_coach';

UPDATE tool_catalog SET
    display_name = 'Get Agent',
    description = 'Get detailed information about a specific agent'
WHERE tool_name = 'get_coach';

UPDATE tool_catalog SET
    display_name = 'Update Agent',
    description = 'Update an existing agent settings'
WHERE tool_name = 'update_coach';

UPDATE tool_catalog SET
    display_name = 'Delete Agent',
    description = 'Delete an agent'
WHERE tool_name = 'delete_coach';

UPDATE tool_catalog SET
    display_name = 'Toggle Agent Favorite',
    description = 'Toggle the favorite status of an agent'
WHERE tool_name = 'toggle_coach_favorite';

UPDATE tool_catalog SET
    display_name = 'Search Agents',
    description = 'Search for agents by query'
WHERE tool_name = 'search_coaches';

UPDATE tool_catalog SET
    display_name = 'Activate Agent',
    description = 'Activate an agent for personalized training guidance'
WHERE tool_name = 'activate_coach';

UPDATE tool_catalog SET
    display_name = 'Deactivate Agent',
    description = 'Deactivate the current agent and return to default AI guidance'
WHERE tool_name = 'deactivate_coach';

UPDATE tool_catalog SET
    display_name = 'Get Active Agent',
    description = 'Get the currently active agent'
WHERE tool_name = 'get_active_coach';

UPDATE tool_catalog SET
    display_name = 'Hide Agent',
    description = 'Hide an agent from listings'
WHERE tool_name = 'hide_coach';

UPDATE tool_catalog SET
    display_name = 'Show Agent',
    description = 'Show a previously hidden agent'
WHERE tool_name = 'show_coach';

UPDATE tool_catalog SET
    display_name = 'List Hidden Agents',
    description = 'List all hidden agents'
WHERE tool_name = 'list_hidden_coaches';

-- Operator tools (category 'admin').
UPDATE tool_catalog SET
    display_name = 'List System Agents',
    description = 'List all system agents in the tenant (admin only)'
WHERE tool_name = 'admin_list_system_coaches';

UPDATE tool_catalog SET
    display_name = 'Create System Agent',
    description = 'Create a new system agent visible to all tenant users (admin only)'
WHERE tool_name = 'admin_create_system_coach';

UPDATE tool_catalog SET
    display_name = 'Get System Agent',
    description = 'Get detailed information about a system agent (admin only)'
WHERE tool_name = 'admin_get_system_coach';

UPDATE tool_catalog SET
    display_name = 'Update System Agent',
    description = 'Update an existing system agent (admin only)'
WHERE tool_name = 'admin_update_system_coach';

UPDATE tool_catalog SET
    display_name = 'Delete System Agent',
    description = 'Delete a system agent and remove all assignments (admin only)'
WHERE tool_name = 'admin_delete_system_coach';

UPDATE tool_catalog SET
    display_name = 'Assign Agent',
    description = 'Assign a system agent to a specific user (admin only)'
WHERE tool_name = 'admin_assign_coach';

UPDATE tool_catalog SET
    display_name = 'Unassign Agent',
    description = 'Remove an agent assignment from a user (admin only)'
WHERE tool_name = 'admin_unassign_coach';

UPDATE tool_catalog SET
    display_name = 'List Agent Assignments',
    description = 'List all assignments for a system agent (admin only)'
WHERE tool_name = 'admin_list_coach_assignments';

-- Chat-callable memory and store tools. `recall_user_memory` and
-- `coach_followup_schedule` keep their display names — only the sentence that
-- named the persona changes.
UPDATE tool_catalog SET
    description = 'Retrieve stored facts the agent has remembered about the user'
WHERE tool_name = 'recall_user_memory';

UPDATE tool_catalog SET
    display_name = 'Add Agent Note',
    description = 'Persist a private agent note about the user'
WHERE tool_name = 'coach_note_add';

UPDATE tool_catalog SET
    description = 'Schedule a future check-in the agent should remember'
WHERE tool_name = 'coach_followup_schedule';

UPDATE tool_catalog SET
    display_name = 'Browse Agent Store',
    description = 'Browse the catalogue of published agents anyone can install'
WHERE tool_name = 'browse_coach_store';

UPDATE tool_catalog SET
    display_name = 'Search Agent Store',
    description = 'Search the Agent Store for published agents'
WHERE tool_name = 'search_coach_store';

UPDATE tool_catalog SET
    display_name = 'Install Agent from Store',
    description = 'Install a published Agent Store agent into the athlete''s own library'
WHERE tool_name = 'install_coach_from_store';
