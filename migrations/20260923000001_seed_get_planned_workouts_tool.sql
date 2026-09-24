-- ABOUTME: Seeds the tool_catalog row for get_planned_workouts, the provider-neutral planned-calendar read
-- ABOUTME: Catalogued so a tenant can disable it; enabled on the starter plan like get_activities, whose data it sits beside
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- get_planned_workouts registers in the `data` category, which
-- `chat_callable_schemas` offers the athlete-facing agent, so the agent can
-- read what the athlete's coach planned on the turn it is asked. An
-- uncatalogued tool is always enabled and cannot be disabled per tenant
-- (carnet#143), and the catalog completeness test holds the seeded set to the
-- registry, so the row lands with the tool.
--
-- Same shape as get_activities (tc-001): category `fitness`, enabled by
-- default, `starter`, requires_provider NULL like every comparable row — the
-- tool picks the connected provider with a planned calendar itself. tc-145 is
-- the next id after tc-144, the highest any migration claims. INSERT OR IGNORE:
-- a database that already booted holds a synthesised `tc-auto-` row under this
-- name and keeps it, with any text an operator edited.

INSERT OR IGNORE INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-145', 'get_planned_workouts', 'Get Planned Workouts', 'Read the workouts a connected provider''s training calendar plans for the athlete', 'fitness', 1, NULL, 'starter');
