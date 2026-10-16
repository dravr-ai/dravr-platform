-- ABOUTME: Seeds the tool_catalog row for import_provider_thresholds, the athlete's thresholds from a provider's settings
-- ABOUTME: Catalogued so a tenant can disable it; enabled on the starter plan like set_physiology
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- import_provider_thresholds reads the FTP, threshold heart rate, max heart
-- rate and threshold pace the athlete keeps in intervals.icu's sport
-- settings and fills the physiological profile with them where the athlete
-- has not set their own (carnet#1021). It registers in the `physiology`
-- category beside set_physiology, which `chat_callable_schemas` offers the
-- agent. An uncatalogued tool is always enabled and cannot be disabled per
-- tenant (carnet#143), and the catalog completeness test holds the seeded set
-- to the registry, so the row lands with the tool.
--
-- The catalog category is not the registry one: the CHECK admits
-- `configuration`, which is what boot sync derives for a `physiology` tool,
-- and what set_physiology (tc-095) is seeded under. Enabled by default,
-- `starter`, requires_provider NULL. tc-150 is the next id after tc-149, the
-- highest any migration claims. ON CONFLICT DO NOTHING: a database that already
-- booted holds a synthesised `tc-auto-` row under the name and keeps it, with
-- any text an operator edited.

INSERT INTO tool_catalog (id, tool_name, display_name, description, category, is_enabled_by_default, requires_provider, min_plan) VALUES
('tc-150', 'import_provider_thresholds', 'Import Provider Thresholds', 'Fill the athlete''s FTP, threshold and max heart rate and threshold pace from their intervals.icu sport settings, never replacing a value the athlete set', 'configuration', TRUE, NULL, 'starter')
ON CONFLICT (tool_name) DO NOTHING;
