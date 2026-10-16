-- ABOUTME: Seeds the tool_catalog rows for get_provider_roster and propose_athlete_link, the coaching-platform roster from chat
-- ABOUTME: Catalogued so a tenant can disable them; enabled on the starter plan like get_roster_overview
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- get_provider_roster reads a human coach's coaching-platform roster
-- (Intervals.icu, TrainingPeaks) with where each athlete stands with the
-- coach's Dravr groups, and propose_athlete_link asks a member to confirm a
-- link to their roster entry (carnet#1018). Both register in the `groups`
-- category, which `chat_callable_schemas` offers the agent and the coach seat
-- keeps. An uncatalogued tool is always enabled and cannot be disabled per
-- tenant (carnet#143), and the catalog completeness test holds the seeded set
-- to the registry, so the rows land with the tools.
--
-- The catalog category is not the registry one: the CHECK admits `fitness`,
-- which is what boot sync derives for a `groups` tool declaring neither
-- ADMIN_ONLY nor REQUIRES_PROVIDER. Same shape as get_roster_overview
-- (tc-146): category `fitness`, enabled by default, `starter`,
-- requires_provider NULL. tc-148 and tc-149 are the next ids after tc-147,
-- the highest any migration claims. INSERT OR IGNORE: a database that already
-- booted holds a synthesised `tc-auto-` row under each name and keeps it,
-- with any text an operator edited.

INSERT OR IGNORE INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-148', 'get_provider_roster', 'Provider Roster', 'Read the coach''s coaching-platform roster and where each athlete stands with the coach''s groups, naming Dravr members only', 'fitness', 1, NULL, 'starter'),
('tc-149', 'propose_athlete_link', 'Propose Athlete Link', 'Ask a member of a group the coach coaches to confirm a link to their coaching-platform roster entry', 'fitness', 1, NULL, 'starter');
