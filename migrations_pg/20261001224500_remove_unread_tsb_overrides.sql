-- ABOUTME: Removes overrides of the retired tsb.fatigued_threshold and tsb.fresh_min catalog keys (PostgreSQL version)
-- ABOUTME: Nothing ever read them; the recovery score now bands form as a share of CTL in dravr-cageux

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The admin catalog offered two absolute-TSB thresholds under the
-- training_stress category, but no code mapped them onto the cageux
-- intelligence config: an override of either changed no athlete's recovery
-- score. dravr-cageux now scores the load component on form as a share of
-- the athlete's own CTL, banded by FormBand, and has no absolute TSB
-- thresholds left to configure, so the catalog entries are retired. A row
-- left behind would be an override with no definition; removing it changes
-- nothing an athlete sees.
DELETE FROM admin_config_overrides
WHERE category = 'training_stress'
  AND config_key IN ('tsb.fatigued_threshold', 'tsb.fresh_min');
