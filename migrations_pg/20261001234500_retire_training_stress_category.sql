-- ABOUTME: Retires the admin config training_stress category, left empty once its two TSB keys were removed (PostgreSQL version)
-- ABOUTME: Drops any override still filed under it and the category row, so the console shows no empty section

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- tsb.fatigued_threshold and tsb.fresh_min were the training_stress
-- category's only parameters; 20261001224500 removed their overrides when the
-- catalog entries went away (dravr-cageux scores recovery load on form as a
-- share of CTL and has no absolute TSB thresholds to configure). The category
-- row seeded as cat_tsb would otherwise render as an empty "Training Stress"
-- section offering thresholds that no longer exist. Both deletes match on the
-- category name, so they are safe to run whatever the earlier one left.
DELETE FROM admin_config_overrides WHERE category = 'training_stress';
DELETE FROM admin_config_categories WHERE name = 'training_stress';
