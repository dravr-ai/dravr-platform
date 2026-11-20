-- ABOUTME: The sciotte_reads config category, backing the per-athlete session-read caps by tier
-- ABOUTME: sciotte_reads.<cap>.<tier>; unset, they read the compiled SessionReadCaps (carnet#1661)
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The catalog lists a parameter only under a category it has a row for, so
-- without this one `pierre-cli config show` and `config get` cannot see the
-- fifteen sciotte_reads.* parameters.
INSERT INTO admin_config_categories (id, name, display_name, description, display_order, icon) VALUES
    ('sciotte_reads', 'sciotte_reads', 'Session Read Caps', 'How many reads of an athlete''s signed-in Strava, Garmin, COROS or TrainingPeaks session Dravr makes per day and per 15 minutes, by tier', 161, 'clock')
ON CONFLICT (id) DO NOTHING;
