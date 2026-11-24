-- ABOUTME: Retires the eight admin config categories that hold no key, five of them emptied by 22 unread keys
-- ABOUTME: Drops their overrides and those of every removed key, so the console shows no empty section

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Twenty-two catalog keys left the code because nothing read them: an
-- admin could override any of them and nothing would change. Five
-- categories held only those keys (feature_flags, recommendation_engine,
-- sleep_recovery, weather_analysis, monitoring). Three more were seeded with
-- no key ever defined for them (heart_rate_zones, effort_thresholds,
-- cache_ttl). Every one of these rows was seeded by a migration, so each
-- would otherwise render as an empty section in the admin console.
-- algorithm.tss and the three Garmin rate-limit keys sat in categories that
-- still define other keys, so those categories stay and only the stored
-- overrides of the removed keys go. The runtime feature flags are
-- a separate table and are not touched. Every delete matches on names, so
-- the migration is safe to run whatever an earlier one left.
DELETE FROM admin_config_overrides
WHERE category IN (
        'feature_flags',
        'recommendation_engine',
        'sleep_recovery',
        'weather_analysis',
        'monitoring',
        'heart_rate_zones',
        'effort_thresholds',
        'cache_ttl'
    )
   OR config_key IN (
        'feature.auto_approval_enabled',
        'algorithm.tss',
        'recommendation.low_weekly_distance_km',
        'recommendation.high_weekly_distance_km',
        'recommendation.max_per_category',
        'sleep.adult_min_hours',
        'sleep.deep_sleep_min_percent',
        'weather.ideal_min_celsius',
        'weather.ideal_max_celsius',
        'provider.garmin_rate_limit_hourly',
        'provider.garmin_rate_limit_daily',
        'provider.garmin_rate_limit_block_secs',
        'monitoring.memory_warn_mb',
        'monitoring.memory_critical_mb',
        'monitoring.latency_warn_ms',
        'monitoring.latency_critical_ms',
        'monitoring.error_rate_warn_pct',
        'monitoring.error_rate_critical_pct',
        'monitoring.pool_usage_warn_pct',
        'monitoring.pool_usage_critical_pct',
        'monitoring.cache_hit_rate_warn_pct',
        'monitoring.cache_miss_rate_critical_pct'
    );
DELETE FROM admin_config_categories
WHERE name IN (
        'feature_flags',
        'recommendation_engine',
        'sleep_recovery',
        'weather_analysis',
        'monitoring',
        'heart_rate_zones',
        'effort_thresholds',
        'cache_ttl'
    );
