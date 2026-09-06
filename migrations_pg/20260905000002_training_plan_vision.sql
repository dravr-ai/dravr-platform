-- ABOUTME: The plan outline grows into the vision — phases on the kernel's nine kinds, a flavour, a season window
-- ABOUTME: blocks_json becomes phases_json with `phase` renamed `kind` (rest -> recovery); weeks gain phase_index
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'training_plans' AND column_name = 'blocks_json'
    ) THEN
        ALTER TABLE training_plans RENAME COLUMN blocks_json TO phases_json;
    END IF;
END $$;
ALTER TABLE training_plans ADD COLUMN IF NOT EXISTS flavour_json TEXT;
ALTER TABLE training_plans ADD COLUMN IF NOT EXISTS season_start TEXT;
ALTER TABLE training_plans ADD COLUMN IF NOT EXISTS season_end TEXT;
ALTER TABLE training_plan_weeks ADD COLUMN IF NOT EXISTS phase_index INTEGER;

-- Every stored block becomes a phase: `phase` is renamed `kind`, and the old
-- `rest` kind is the kernel's `recovery`. The other four names are unchanged.
UPDATE training_plans
SET phases_json = (
    SELECT COALESCE(jsonb_agg(
        (elem - 'phase') || jsonb_build_object(
            'kind',
            CASE elem->>'phase' WHEN 'rest' THEN 'recovery' ELSE elem->>'phase' END
        )
    ), '[]'::jsonb)::text
    FROM jsonb_array_elements(phases_json::jsonb) AS elem
)
WHERE jsonb_typeof(phases_json::jsonb) = 'array'
  AND EXISTS (SELECT 1 FROM jsonb_array_elements(phases_json::jsonb) AS e WHERE e ? 'phase');
