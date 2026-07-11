-- Sleep-analysis tools available on every plan
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai
--
-- The 2025 catalog seed gated the sleep-analysis tools at 'professional'
-- ('enterprise' for optimize_sleep_schedule) while the DB-backed
-- get_sleep_sessions/get_recovery_metrics are 'starter'. The plan check in
-- is_tool_enabled runs before tenant overrides, so on sub-professional
-- tenants the coach's sleep-analysis calls were refused as tenant_disabled
-- even though the admin console showed them Enabled/Default. Sleep analysis
-- is core coach functionality during beta — align the whole category on
-- 'starter'.
UPDATE tool_catalog
SET min_plan = 'starter'
WHERE tool_name IN (
    'analyze_sleep_quality',
    'calculate_recovery_score',
    'suggest_rest_day',
    'track_sleep_trends',
    'optimize_sleep_schedule'
);
