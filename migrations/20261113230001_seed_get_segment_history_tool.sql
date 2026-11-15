-- ABOUTME: Seeds the tool_catalog row for get_segment_history, one segment's efforts across the athlete's activities
-- ABOUTME: Catalogued so a tenant can disable it; enabled on the starter plan beside the other activity reads
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- get_segment_history answers every effort the athlete made on one segment,
-- from the provider's live listing or the efforts stored with their activity
-- details, and asks a detail walk for what is unread (carnet#1483). It
-- registers among the data tools (`create_data_tools`), which
-- `chat_callable_schemas` offers the agent. An uncatalogued tool is always
-- enabled and cannot be disabled per tenant (carnet#143), and the catalog
-- completeness test holds the seeded set to the registry, so the row lands
-- with the tool.
--
-- Same shape as get_heart_rate_series (tc-152): category `fitness`, enabled
-- by default, `starter`, requires_provider NULL — the efforts are read from
-- whichever backend holds them. tc-155 is the next id after tc-154, the
-- highest any migration claims. INSERT OR IGNORE: a database that already
-- booted holds a synthesised `tc-auto-` row under this name and keeps it,
-- with any text an operator edited.

INSERT OR IGNORE INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-155', 'get_segment_history', 'Segment History', 'Get every effort the athlete made on one segment, with its count per year and best time', 'fitness', 1, NULL, 'starter');
