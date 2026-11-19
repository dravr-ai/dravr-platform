-- ABOUTME: The activity_landed config category, backing how often the coach reacts to a synced workout
-- ABOUTME: activity_landed.frequency and .digest; unset, they read per_day and replace_digest (carnet#1486)
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The catalog lists a parameter only under a category it has a row for, so
-- without this one `pierre-cli config show` and `config get` cannot see the
-- two activity_landed.* parameters.
INSERT INTO admin_config_categories (id, name, display_name, description, display_order, icon) VALUES
    ('activity_landed', 'activity_landed', 'Coach Landing Posts', 'How often the coach reacts to a newly synced workout, and whether that reaction replaces the per-session digest', 159, 'users')
ON CONFLICT (id) DO NOTHING;
