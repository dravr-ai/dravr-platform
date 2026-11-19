-- ABOUTME: Seeds the tool_catalog row for name_place, which names or renames a place where the athlete trains (PostgreSQL version)
-- ABOUTME: Catalogued so a tenant can disable it; enabled on the starter plan like set_preferred_calendar
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- name_place registers in the `memory` category (carnet#1484, slice 6): the
-- athlete names the place a row's place.id points at, or the place where one
-- activity started, and can clear the name back to the automatic one. An
-- uncatalogued tool is always enabled and cannot be disabled per tenant
-- (carnet#143), and the catalog completeness test holds the seeded set to the
-- registry, so the row lands with the tool.
--
-- Same shape as set_preferred_calendar (tc-154): category `fitness`, enabled
-- by default, `starter`, requires_provider NULL. tc-156 is the next id after
-- tc-155, the highest any migration claims. ON CONFLICT DO NOTHING: a database that
-- already booted holds a synthesised `tc-auto-` row under this name and keeps
-- it, with any text an operator edited.

INSERT INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-156', 'name_place', 'Name Place', 'Name or rename a place where the athlete trains', 'fitness', TRUE, NULL, 'starter') ON CONFLICT (tool_name) DO NOTHING;
