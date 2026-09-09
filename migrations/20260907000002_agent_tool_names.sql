-- ABOUTME: The 26 agent-persona MCP tool names lose the word coach in the tool catalogue
-- ABOUTME: Each tenant and user override follows its tool across, so nobody's disabled tool comes back on
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- ADR-026: the persona an athlete talks to is an agent; a coach is a human.
-- 20260906000001 rewrote the display text these rows carry. This moves the key
-- itself -- `tool_name`, the MCP wire name -- for the 26 tools that operate on
-- the persona.
--
-- Order is the whole design. `tenant_tool_overrides.tool_name` and
-- `user_tool_overrides.tool_name` are
-- `REFERENCES tool_catalog(tool_name) ON DELETE CASCADE`, and neither declares
-- ON UPDATE, so:
--
--   * renaming the catalogue row in place is refused while an override still
--     points at it -- NO ACTION on both backends;
--   * deleting the old row first takes every override for it with it.
--
-- So each tool is carried across in one direction: INSERT the new-named row
-- from the old one, move both override tables onto the new name, and only then
-- DELETE the old row -- by which point nothing references it and the cascade
-- takes nothing.
--
-- This is not optional. Every boot runs `run_tool_catalog_sync`, which deletes
-- any catalogue row the registry no longer registers. Without this migration
-- the first boot after the rename would cascade away every override for these
-- 26 tools, and `guardian::tenant_tool_enabled` reads a missing row as "no
-- override applies" -- a tool an operator had turned off would come back on.
--
-- `category` is untouched: 'coaches' and 'admin' are values the CHECK
-- constraint already admits, and a renamed tool keeps its category. The new
-- row copies every other column from the old one, so display text, plan gate
-- and timestamps carry across unchanged. `id` is the primary key and cannot be
-- reused while the old row is still present, so each new row takes the next
-- free `tc-` id after tc-108.
--
-- `list_coaching_playbooks` is not here. A playbook is a learned pattern about
-- the activity of coaching, not the persona, and it reads the same in every
-- locale. `usage_counters.tool_name` and `guardian_pending_actions.tool_name`
-- are not here either: the first is an accounting dimension over calls that
-- already happened, and the second is a confirmation ledger whose rows carry
-- their own `expires_at`.
--
-- Idempotent for a re-run: the INSERT ignores a row already there, and once the
-- old name is gone the two UPDATEs and the DELETE match nothing. Applying the
-- file twice is a no-op, and a database that never held the old name is
-- untouched.
--
-- One state is deliberately NOT handled, and fails loudly rather than guessing.
-- If the same tenant or user somehow holds an override under BOTH the old and
-- the new name, the UPDATE violates `UNIQUE(tenant_id, tool_name)` (or the
-- user table's composite primary key), the surrounding transaction rolls back,
-- and `Database::new` refuses to start. That state is unreachable by deploying:
-- migrations run inside `Database::new`, before the registry exists, so one
-- boot cannot create it. It takes a restored or partial dump, a cleared
-- `_sqlx_migrations`, or a hand-patched database -- all of which already need a
-- human. Resolving it here would mean silently discarding one of the two
-- overrides, which is the exact harm this migration exists to prevent, so it
-- refuses instead and says so.

-- The athlete's own agent library.

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-109', 'list_agents', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'list_coaches';
UPDATE tenant_tool_overrides SET tool_name = 'list_agents' WHERE tool_name = 'list_coaches';
UPDATE user_tool_overrides SET tool_name = 'list_agents' WHERE tool_name = 'list_coaches';
DELETE FROM tool_catalog WHERE tool_name = 'list_coaches';

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-110', 'create_agent', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'create_coach';
UPDATE tenant_tool_overrides SET tool_name = 'create_agent' WHERE tool_name = 'create_coach';
UPDATE user_tool_overrides SET tool_name = 'create_agent' WHERE tool_name = 'create_coach';
DELETE FROM tool_catalog WHERE tool_name = 'create_coach';

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-111', 'get_agent', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'get_coach';
UPDATE tenant_tool_overrides SET tool_name = 'get_agent' WHERE tool_name = 'get_coach';
UPDATE user_tool_overrides SET tool_name = 'get_agent' WHERE tool_name = 'get_coach';
DELETE FROM tool_catalog WHERE tool_name = 'get_coach';

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-112', 'update_agent', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'update_coach';
UPDATE tenant_tool_overrides SET tool_name = 'update_agent' WHERE tool_name = 'update_coach';
UPDATE user_tool_overrides SET tool_name = 'update_agent' WHERE tool_name = 'update_coach';
DELETE FROM tool_catalog WHERE tool_name = 'update_coach';

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-113', 'delete_agent', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'delete_coach';
UPDATE tenant_tool_overrides SET tool_name = 'delete_agent' WHERE tool_name = 'delete_coach';
UPDATE user_tool_overrides SET tool_name = 'delete_agent' WHERE tool_name = 'delete_coach';
DELETE FROM tool_catalog WHERE tool_name = 'delete_coach';

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-114', 'toggle_agent_favorite', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'toggle_coach_favorite';
UPDATE tenant_tool_overrides SET tool_name = 'toggle_agent_favorite' WHERE tool_name = 'toggle_coach_favorite';
UPDATE user_tool_overrides SET tool_name = 'toggle_agent_favorite' WHERE tool_name = 'toggle_coach_favorite';
DELETE FROM tool_catalog WHERE tool_name = 'toggle_coach_favorite';

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-115', 'search_agents', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'search_coaches';
UPDATE tenant_tool_overrides SET tool_name = 'search_agents' WHERE tool_name = 'search_coaches';
UPDATE user_tool_overrides SET tool_name = 'search_agents' WHERE tool_name = 'search_coaches';
DELETE FROM tool_catalog WHERE tool_name = 'search_coaches';

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-116', 'activate_agent', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'activate_coach';
UPDATE tenant_tool_overrides SET tool_name = 'activate_agent' WHERE tool_name = 'activate_coach';
UPDATE user_tool_overrides SET tool_name = 'activate_agent' WHERE tool_name = 'activate_coach';
DELETE FROM tool_catalog WHERE tool_name = 'activate_coach';

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-117', 'deactivate_agent', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'deactivate_coach';
UPDATE tenant_tool_overrides SET tool_name = 'deactivate_agent' WHERE tool_name = 'deactivate_coach';
UPDATE user_tool_overrides SET tool_name = 'deactivate_agent' WHERE tool_name = 'deactivate_coach';
DELETE FROM tool_catalog WHERE tool_name = 'deactivate_coach';

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-118', 'get_active_agent', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'get_active_coach';
UPDATE tenant_tool_overrides SET tool_name = 'get_active_agent' WHERE tool_name = 'get_active_coach';
UPDATE user_tool_overrides SET tool_name = 'get_active_agent' WHERE tool_name = 'get_active_coach';
DELETE FROM tool_catalog WHERE tool_name = 'get_active_coach';

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-119', 'hide_agent', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'hide_coach';
UPDATE tenant_tool_overrides SET tool_name = 'hide_agent' WHERE tool_name = 'hide_coach';
UPDATE user_tool_overrides SET tool_name = 'hide_agent' WHERE tool_name = 'hide_coach';
DELETE FROM tool_catalog WHERE tool_name = 'hide_coach';

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-120', 'show_agent', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'show_coach';
UPDATE tenant_tool_overrides SET tool_name = 'show_agent' WHERE tool_name = 'show_coach';
UPDATE user_tool_overrides SET tool_name = 'show_agent' WHERE tool_name = 'show_coach';
DELETE FROM tool_catalog WHERE tool_name = 'show_coach';

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-121', 'list_hidden_agents', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'list_hidden_coaches';
UPDATE tenant_tool_overrides SET tool_name = 'list_hidden_agents' WHERE tool_name = 'list_hidden_coaches';
UPDATE user_tool_overrides SET tool_name = 'list_hidden_agents' WHERE tool_name = 'list_hidden_coaches';
DELETE FROM tool_catalog WHERE tool_name = 'list_hidden_coaches';

-- Operator tools over the tenant's system agents and their assignments.

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-122', 'admin_list_system_agents', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'admin_list_system_coaches';
UPDATE tenant_tool_overrides SET tool_name = 'admin_list_system_agents' WHERE tool_name = 'admin_list_system_coaches';
UPDATE user_tool_overrides SET tool_name = 'admin_list_system_agents' WHERE tool_name = 'admin_list_system_coaches';
DELETE FROM tool_catalog WHERE tool_name = 'admin_list_system_coaches';

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-123', 'admin_create_system_agent', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'admin_create_system_coach';
UPDATE tenant_tool_overrides SET tool_name = 'admin_create_system_agent' WHERE tool_name = 'admin_create_system_coach';
UPDATE user_tool_overrides SET tool_name = 'admin_create_system_agent' WHERE tool_name = 'admin_create_system_coach';
DELETE FROM tool_catalog WHERE tool_name = 'admin_create_system_coach';

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-124', 'admin_get_system_agent', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'admin_get_system_coach';
UPDATE tenant_tool_overrides SET tool_name = 'admin_get_system_agent' WHERE tool_name = 'admin_get_system_coach';
UPDATE user_tool_overrides SET tool_name = 'admin_get_system_agent' WHERE tool_name = 'admin_get_system_coach';
DELETE FROM tool_catalog WHERE tool_name = 'admin_get_system_coach';

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-125', 'admin_update_system_agent', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'admin_update_system_coach';
UPDATE tenant_tool_overrides SET tool_name = 'admin_update_system_agent' WHERE tool_name = 'admin_update_system_coach';
UPDATE user_tool_overrides SET tool_name = 'admin_update_system_agent' WHERE tool_name = 'admin_update_system_coach';
DELETE FROM tool_catalog WHERE tool_name = 'admin_update_system_coach';

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-126', 'admin_delete_system_agent', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'admin_delete_system_coach';
UPDATE tenant_tool_overrides SET tool_name = 'admin_delete_system_agent' WHERE tool_name = 'admin_delete_system_coach';
UPDATE user_tool_overrides SET tool_name = 'admin_delete_system_agent' WHERE tool_name = 'admin_delete_system_coach';
DELETE FROM tool_catalog WHERE tool_name = 'admin_delete_system_coach';

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-127', 'admin_assign_agent', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'admin_assign_coach';
UPDATE tenant_tool_overrides SET tool_name = 'admin_assign_agent' WHERE tool_name = 'admin_assign_coach';
UPDATE user_tool_overrides SET tool_name = 'admin_assign_agent' WHERE tool_name = 'admin_assign_coach';
DELETE FROM tool_catalog WHERE tool_name = 'admin_assign_coach';

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-128', 'admin_unassign_agent', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'admin_unassign_coach';
UPDATE tenant_tool_overrides SET tool_name = 'admin_unassign_agent' WHERE tool_name = 'admin_unassign_coach';
UPDATE user_tool_overrides SET tool_name = 'admin_unassign_agent' WHERE tool_name = 'admin_unassign_coach';
DELETE FROM tool_catalog WHERE tool_name = 'admin_unassign_coach';

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-129', 'admin_list_agent_assignments', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'admin_list_coach_assignments';
UPDATE tenant_tool_overrides SET tool_name = 'admin_list_agent_assignments' WHERE tool_name = 'admin_list_coach_assignments';
UPDATE user_tool_overrides SET tool_name = 'admin_list_agent_assignments' WHERE tool_name = 'admin_list_coach_assignments';
DELETE FROM tool_catalog WHERE tool_name = 'admin_list_coach_assignments';

-- The Agent Store an athlete installs a published agent from.

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-130', 'browse_agent_store', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'browse_coach_store';
UPDATE tenant_tool_overrides SET tool_name = 'browse_agent_store' WHERE tool_name = 'browse_coach_store';
UPDATE user_tool_overrides SET tool_name = 'browse_agent_store' WHERE tool_name = 'browse_coach_store';
DELETE FROM tool_catalog WHERE tool_name = 'browse_coach_store';

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-131', 'search_agent_store', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'search_coach_store';
UPDATE tenant_tool_overrides SET tool_name = 'search_agent_store' WHERE tool_name = 'search_coach_store';
UPDATE user_tool_overrides SET tool_name = 'search_agent_store' WHERE tool_name = 'search_coach_store';
DELETE FROM tool_catalog WHERE tool_name = 'search_coach_store';

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-132', 'install_agent_from_store', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'install_coach_from_store';
UPDATE tenant_tool_overrides SET tool_name = 'install_agent_from_store' WHERE tool_name = 'install_coach_from_store';
UPDATE user_tool_overrides SET tool_name = 'install_agent_from_store' WHERE tool_name = 'install_coach_from_store';
DELETE FROM tool_catalog WHERE tool_name = 'install_coach_from_store';

-- What the agent remembers about the athlete between turns.

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-133', 'agent_note_add', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'coach_note_add';
UPDATE tenant_tool_overrides SET tool_name = 'agent_note_add' WHERE tool_name = 'coach_note_add';
UPDATE user_tool_overrides SET tool_name = 'agent_note_add' WHERE tool_name = 'coach_note_add';
DELETE FROM tool_catalog WHERE tool_name = 'coach_note_add';

INSERT OR IGNORE INTO tool_catalog (
    id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at)
SELECT 'tc-134', 'agent_followup_schedule', display_name, description, category, is_enabled_by_default, requires_provider, min_plan, created_at, updated_at
FROM tool_catalog WHERE tool_name = 'coach_followup_schedule';
UPDATE tenant_tool_overrides SET tool_name = 'agent_followup_schedule' WHERE tool_name = 'coach_followup_schedule';
UPDATE user_tool_overrides SET tool_name = 'agent_followup_schedule' WHERE tool_name = 'coach_followup_schedule';
DELETE FROM tool_catalog WHERE tool_name = 'coach_followup_schedule';
