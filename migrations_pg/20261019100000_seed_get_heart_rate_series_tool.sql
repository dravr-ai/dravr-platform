-- ABOUTME: Seeds the tool_catalog row for get_heart_rate_series, the stored intraday heart rate and HRV read
-- ABOUTME: Catalogued so a tenant can disable it; enabled on the starter plan beside the stored health reads
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- get_heart_rate_series reads back the continuous series the health sync
-- stores in data_point_series through riviere's TimeSeriesStore, which had no
-- reader until now (carnet#999). It registers among the data tools
-- (`create_data_tools`) beside the stored health reads, which
-- `chat_callable_schemas` offers the agent. An uncatalogued tool is always
-- enabled and cannot be disabled per tenant (carnet#143), and the catalog
-- completeness test holds the seeded set to the registry, so the row lands
-- with the tool.
--
-- Same shape as get_health_snapshots (tc-078): category `fitness`, enabled by default,
-- `starter`, requires_provider NULL — the series comes from whichever
-- connected source synced it. tc-152 is the next id after tc-151, the highest
-- any migration claims. ON CONFLICT on tool_name: a database that already
-- booted holds a synthesised `tc-auto-` row under this name and keeps it, with
-- any text an operator edited.

INSERT INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-152', 'get_heart_rate_series', 'Heart Rate Series', 'Get stored intraday heart rate and HRV samples, raw or aggregated per window', 'fitness', TRUE, NULL, 'starter')
ON CONFLICT (tool_name) DO NOTHING;
