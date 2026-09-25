-- ABOUTME: Retires the seed rows that still name the deleted Fitbit Web API provider
-- ABOUTME: Drops its admin config category and overrides; rewords the sleep-quality catalogue entry

DELETE FROM admin_config_overrides WHERE category = 'provider_fitbit';
DELETE FROM admin_config_categories WHERE name = 'provider_fitbit';

UPDATE tool_catalog
SET description = 'Analyze sleep quality from WHOOP/Garmin data using NSF/AASM guidelines'
WHERE tool_name = 'analyze_sleep_quality'
  AND description = 'Analyze sleep quality from Fitbit/Garmin data using NSF/AASM guidelines';
