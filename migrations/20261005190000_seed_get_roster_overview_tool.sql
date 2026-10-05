-- ABOUTME: Seeds the tool_catalog row for get_roster_overview, the coach's roster review from their own thread
-- ABOUTME: Catalogued so a tenant can disable it; enabled on the starter plan like get_group_member_activities
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- get_roster_overview registers in the `groups` category, which
-- `chat_callable_schemas` offers the agent and the coach seat keeps, so the
-- roster agent can review every athlete a coach holds from the coach's own
-- thread (registre#748). An uncatalogued tool is always enabled and cannot be
-- disabled per tenant (carnet#143), and the catalog completeness test holds
-- the seeded set to the registry, so the row lands with the tool.
--
-- Same shape as get_group_member_activities (tc-094): category `fitness`,
-- enabled by default, `starter`, requires_provider NULL. tc-146 is the next id
-- after tc-145, the highest any migration claims. INSERT OR IGNORE: a database
-- that already booted holds a synthesised `tc-auto-` row under this name and
-- keeps it, with any text an operator edited.

INSERT OR IGNORE INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-146', 'get_roster_overview', 'Roster Overview', 'Review the training of every athlete the coach holds, each by their consent to share with the coach', 'fitness', 1, NULL, 'starter');
