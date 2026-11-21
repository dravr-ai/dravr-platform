-- ABOUTME: Seeds the tool_catalog row for get_health_records, the athlete's lab results, clinical records, glucose and cycle tracking
-- ABOUTME: Catalogued so a tenant can disable it; enabled on the starter plan like get_heart_rate_series
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- get_health_records registers among the data tools (carnet#1544, owl phase 5
-- slice 2): every row it answers is gated by its own tier, and an outside
-- client is refused. An uncatalogued tool is always enabled and cannot be
-- disabled per tenant (carnet#143), and the catalog completeness test holds
-- the seeded set to the registry, so the row lands with the tool.
--
-- Same shape as get_heart_rate_series (tc-152): category `fitness`, enabled by
-- default, `starter`, requires_provider NULL. tc-158 is the next id after
-- tc-157, the highest any migration on main claims. INSERT OR IGNORE: a
-- database that already booted holds a synthesised `tc-auto-` row under this
-- name and keeps it, with any text an operator edited.

INSERT OR IGNORE INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-158', 'get_health_records', 'Health Records', 'Get the athlete''s lab results, clinical records, glucose and cycle tracking, each row under its own sensitivity tier', 'fitness', 1, NULL, 'starter');
