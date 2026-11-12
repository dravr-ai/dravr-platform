-- ABOUTME: The activity_retention config category, backing the one retention policy of provider data
-- ABOUTME: activity_retention.default_days and .provider_days; both start unset, so nothing expires (carnet#1482)
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The catalog lists a parameter only under a category it has a row for, so
-- without this one `pierre-cli config show` and `config get` cannot see the
-- two activity_retention.* parameters.
INSERT INTO admin_config_categories (id, name, display_name, description, display_order, icon) VALUES
    ('activity_retention', 'activity_retention', 'Provider Data Retention', 'How long each provider''s activities and wellness are kept after the last read', 158, 'clock')
ON CONFLICT (id) DO NOTHING;
