-- ABOUTME: Seeds the tool_catalog row for save_workout_to_library, a plan session kept as a template in the caller's own workout library
-- ABOUTME: Catalogued so a tenant can disable it; enabled on the starter plan like push_training_plan
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- save_workout_to_library registers next to the plan tools (carnet#1726): a
-- coach keeps a session of an athlete's plan in the coach's own Intervals.icu
-- library. An uncatalogued tool is always enabled and cannot be disabled per
-- tenant (carnet#143), and the catalog completeness test holds the seeded set
-- to the registry, so the row lands with the tool.
--
-- Same shape as push_training_plan (tc-103): category `fitness`, enabled by
-- default, `starter`, requires_provider NULL. tc-161 is the next id after
-- tc-160, the highest any migration on main claims. INSERT OR IGNORE: a
-- database that already booted holds a synthesised `tc-auto-` row under this
-- name and keeps it, with any text an operator edited.

INSERT OR IGNORE INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-161', 'save_workout_to_library', 'Save Workout to Library', 'Keep one session of a training plan as a template in the caller''s own provider workout library', 'fitness', 1, NULL, 'starter');
