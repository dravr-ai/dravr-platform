-- ABOUTME: Seeds the tool_catalog row for compose_route, which joins discovered trail stretches into one itinerary
-- ABOUTME: Catalogued so a tenant can disable it; enabled on the starter plan like discover_routes
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- compose_route registers beside discover_routes in the `analytics` category,
-- which `chat_callable_schemas` offers the agent: it joins the stretches
-- discover_routes found into one line the chat map draws and whose measured
-- length the agent quotes. An uncatalogued tool is always enabled and cannot
-- be disabled per tenant (carnet#143), and the catalog completeness test
-- holds the seeded set to the registry, so the row lands with the tool.
--
-- Same shape as discover_routes (tc-092): category `analysis`, enabled by
-- default, `starter`, requires_provider NULL. tc-147 is the next id after
-- tc-146, the highest any migration claims. ON CONFLICT DO NOTHING: a
-- database that already booted holds a synthesised `tc-auto-` row under this
-- name and keeps it, with any text an operator edited.

INSERT INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-147', 'compose_route', 'Compose Route', 'Join discovered route stretches that meet into one itinerary, measured and drawn as a single line', 'analysis', TRUE, NULL, 'starter')
ON CONFLICT (tool_name) DO NOTHING;
