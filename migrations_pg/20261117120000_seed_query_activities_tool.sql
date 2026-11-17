-- ABOUTME: Seeds the tool_catalog row for query_activities, which counts and sums the athlete's workouts over any period (PostgreSQL version)
-- ABOUTME: Catalogued so a tenant can disable it; enabled on the starter plan like name_place
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- query_activities registers among the data tools (carnet#1485, slice 7):
-- every count, total and breakdown over a period, from one read of the store,
-- merged across sources. An uncatalogued tool is always enabled and cannot be
-- disabled per tenant (carnet#143), and the catalog completeness test holds
-- the seeded set to the registry, so the row lands with the tool.
--
-- Same shape as name_place (tc-156): category `fitness`, enabled by default,
-- `starter`, requires_provider NULL. tc-157 is the next id after tc-156, the
-- highest any migration on main claims. ON CONFLICT DO NOTHING: a database
-- that already booted holds a synthesised `tc-auto-` row under this name and
-- keeps it, with any text an operator edited.

INSERT INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-157', 'query_activities', 'Query Activities', 'Count and sum the athlete''s activities over any period, by year, month, week, sport, place or gear', 'fitness', TRUE, NULL, 'starter') ON CONFLICT (tool_name) DO NOTHING;
