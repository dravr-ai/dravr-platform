-- ABOUTME: Moves the last analysis tools seeded above starter to the starter plan, so a starter tenant can call them
-- ABOUTME: Plans differ by usage limits, not by tool; these eight were left gated by their seed rows
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- The 2025 catalog seed gated seven of these tools at 'professional'
-- ('enterprise' for predict_performance), and 20260407000001 seeded
-- analyze_weather_impact at 'professional'. A self-serve athlete's tenant is
-- created on 'starter' (tenant_admin::create_tenant_for_user), and the plan
-- check in is_tool_enabled runs on every transport, so in-app chat, /mcp and
-- A2A all refused them with "not available for your tenant" (carnet#1587).
-- 20260711000001 moved the sleep tools for the same reason. After this one no
-- seeded tool sits above 'starter': the plans differ by usage limits, not by
-- tool.
UPDATE tool_catalog
SET min_plan = 'starter'
WHERE tool_name IN (
    'analyze_goal_feasibility',
    'analyze_performance_trends',
    'analyze_training_load',
    'analyze_weather_impact',
    'detect_patterns',
    'generate_recommendations',
    'get_nutrient_timing',
    'predict_performance'
);
