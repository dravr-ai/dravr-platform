-- ABOUTME: Seeds the tool_catalog row for publish_training_plan, a human coach's draft of an athlete's plan made live or discarded
-- ABOUTME: Catalogued so a tenant can disable it; enabled on the starter plan beside push_training_plan
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- A group's human coach's save_training_plan for a coached athlete writes the
-- coach's draft, which the athlete never sees; publish_training_plan makes it
-- the athlete's live plan, or throws it away (carnet#1069). It registers in
-- the `memory` category beside the plan tools, which `chat_callable_schemas`
-- offers the agent. An uncatalogued tool is always enabled and cannot be
-- disabled per tenant (carnet#143), and the catalog completeness test holds
-- the seeded set to the registry, so the row lands with the tool.
--
-- Seeded under `fitness`, as push_training_plan (tc-103) is. Enabled by
-- default, `starter`, requires_provider NULL: publishing writes Dravr's plan,
-- never a provider. tc-151 is the next id after tc-150, the highest any
-- migration claims. INSERT OR IGNORE: a database that already booted holds a
-- synthesised `tc-auto-` row under the name and keeps it, with any text an
-- operator edited.

INSERT OR IGNORE INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-151', 'publish_training_plan', 'Publish Training Plan', 'Publish or discard a group coach''s draft of a coached athlete''s training plan', 'fitness', 1, NULL, 'starter');
