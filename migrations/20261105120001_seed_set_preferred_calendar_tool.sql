-- ABOUTME: Seeds the tool_catalog row for set_preferred_calendar, which records the calendar Dravr's pushes go to
-- ABOUTME: Catalogued so a tenant can disable it; enabled on the starter plan like push_training_plan
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- set_preferred_calendar registers beside push_training_plan in the `memory`
-- category: the agent records the athlete's answer when a push with several
-- calendars connected asks which one, or when they ask to switch
-- (carnet#1393). An uncatalogued tool is always enabled and cannot be
-- disabled per tenant (carnet#143), and the catalog completeness test holds
-- the seeded set to the registry, so the row lands with the tool.
--
-- Same shape as push_training_plan (tc-103): category `fitness`, enabled by
-- default, `starter`, requires_provider NULL. tc-154 is the next id after
-- tc-153, the highest any migration claims. INSERT OR IGNORE: a database that
-- already booted holds a synthesised `tc-auto-` row under this name and keeps
-- it, with any text an operator edited.

INSERT OR IGNORE INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-154', 'set_preferred_calendar', 'Set Preferred Calendar', 'Record which connected calendar Dravr writes the athlete''s plan and workouts to', 'fitness', 1, NULL, 'starter');
