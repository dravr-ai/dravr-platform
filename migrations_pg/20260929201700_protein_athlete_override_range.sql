-- ABOUTME: Removes athlete-protein overrides stored outside the 1.2-2.0 g/kg/day calculate_daily_nutrition now enforces (PostgreSQL version)
-- ABOUTME: nutrition.protein_athlete_g_per_kg gained its reader and narrowed from 1.4-2.5; rows inside the range stay and apply

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Until this migration nothing read nutrition.protein_athlete_g_per_kg: an
-- override of it changed no athlete's protein target. It now sets the athlete
-- target calculate_daily_nutrition prescribes, bounded to the 1.2-2.0 g/kg/day
-- of the ACSM / Academy of Nutrition and Dietetics / Dietitians of Canada
-- joint position statement (Thomas, Erdman & Burke 2016), and the reader
-- refuses a stored value outside that range rather than prescribe it.
--
-- The catalogue used to accept 1.4-2.5, so a row written then may sit above
-- 2.0; left in place it would fail the tool for every athlete it covers. Such
-- a row never took effect, so removing it leaves those athletes on the target
-- they have been getting all along: the kernel's. A row inside the range
-- stays and now applies, which is what the operator who wrote it asked for.
--
-- config_value holds the JSON-encoded number. The cast runs only on a value
-- shaped like one, so a row that is not a number cannot abort the migration;
-- such a row is outside the range as well, and the reader refuses it the same
-- way.
DELETE FROM admin_config_overrides
WHERE category = 'nutrition'
  AND config_key = 'nutrition.protein_athlete_g_per_kg'
  AND NOT COALESCE(
      CASE
          WHEN config_value ~ '^\s*-?[0-9]+(\.[0-9]+)?([eE][-+]?[0-9]+)?\s*$'
          THEN CAST(config_value AS DOUBLE PRECISION) BETWEEN 1.2 AND 2.0
      END,
      FALSE
  );
