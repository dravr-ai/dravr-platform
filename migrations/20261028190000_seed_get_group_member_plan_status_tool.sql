-- ABOUTME: Seeds the tool_catalog row for get_group_member_plan_status, a coached athlete's plan status readable in the coach's room
-- ABOUTME: Catalogued so a tenant can disable it; enabled on the starter plan like get_roster_overview
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- get_group_member_plan_status registers in the `groups` category, which
-- `chat_callable_schemas` offers the agent and the coach seat keeps, so a
-- coach asking for an athlete's plan in their group's room learns whether
-- one is published and where to read it, instead of a refusal (carnet#1244).
-- An uncatalogued tool is always enabled and cannot be disabled per tenant
-- (carnet#143), and the catalog completeness test holds the seeded set to the
-- registry, so the row lands with the tool.
--
-- Same shape as get_roster_overview (tc-146): category `fitness`, enabled by
-- default, `starter`, requires_provider NULL. tc-153 is the next id after
-- tc-152, the highest any migration claims. INSERT OR IGNORE: a database that
-- already booted holds a synthesised `tc-auto-` row under this name and keeps
-- it, with any text an operator edited.

INSERT OR IGNORE INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-153', 'get_group_member_plan_status', 'Athlete Plan Status', 'Whether a coached athlete has a published training plan, its goal race and season, and where the coach reads it', 'fitness', 1, NULL, 'starter');
