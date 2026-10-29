-- ABOUTME: Seeds the tool_catalog rows for the fuel-product bank's athlete tools: search_fuel_products, add_fuel_item, list_fuel_items, remove_fuel_item
-- ABOUTME: Catalogued so a tenant can disable them; enabled on the starter plan like search_food
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- The four tools register in the `nutrition` category, which
-- `chat_callable_schemas` offers the agent: an athlete searches the fuel
-- catalogue and keeps the list of gels, drink mixes and bars they carry, and
-- files a product the catalogue lacks for review (carnet#155). An
-- uncatalogued tool is always enabled and cannot be disabled per tenant
-- (carnet#143), and the catalog completeness test holds the seeded set to
-- the registry, so the rows land with the tools.
--
-- Same shape as search_food (tc-038): category `nutrition`, enabled by
-- default, `starter`, requires_provider NULL. tc-154 to tc-157 follow tc-153,
-- the highest any migration claims. ON CONFLICT DO NOTHING: a database that
-- already booted holds a synthesised `tc-auto-` row under each name and keeps it,
-- with any text an operator edited.

INSERT INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-154', 'search_fuel_products', 'Search Fuel Products', 'Search the fuel-product catalogue by name or barcode for approved label facts per serving', 'nutrition', TRUE, NULL, 'starter'),
('tc-155', 'add_fuel_item', 'Add Fuel Item', 'Add a catalogue product to the athlete''s fuel list, or file one the catalogue lacks for review', 'nutrition', TRUE, NULL, 'starter'),
('tc-156', 'list_fuel_items', 'List Fuel Items', 'The fuel products the athlete carries: approved with label facts, pending review, or rejected', 'nutrition', TRUE, NULL, 'starter'),
('tc-157', 'remove_fuel_item', 'Remove Fuel Item', 'Take a product off the athlete''s fuel list', 'nutrition', TRUE, NULL, 'starter')
ON CONFLICT (tool_name) DO NOTHING;
