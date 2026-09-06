-- ABOUTME: The plan outline grows into the vision — phases on the kernel's nine kinds, a flavour, a season window
-- ABOUTME: blocks_json becomes phases_json with `phase` renamed `kind` (rest -> recovery); weeks gain phase_index
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

ALTER TABLE training_plans RENAME COLUMN blocks_json TO phases_json;  -- idempotency-ok: SQLite RENAME COLUMN has no IF EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE training_plans ADD COLUMN flavour_json TEXT;  -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE training_plans ADD COLUMN season_start TEXT;  -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE training_plans ADD COLUMN season_end TEXT;  -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run
ALTER TABLE training_plan_weeks ADD COLUMN phase_index INTEGER;  -- idempotency-ok: SQLite ADD COLUMN has no IF NOT EXISTS; _sqlx_migrations prevents re-run

-- Every stored block becomes a phase: `phase` is renamed `kind`, and the old
-- `rest` kind is the kernel's `recovery`. The other four names are unchanged.
-- json() re-parses each rewritten element, otherwise json_group_array would
-- store it as an escaped string.
UPDATE training_plans
SET phases_json = (
    SELECT json_group_array(json(json_set(
        json_remove(value, '$.phase'),
        '$.kind',
        CASE json_extract(value, '$.phase') WHEN 'rest' THEN 'recovery' ELSE json_extract(value, '$.phase') END
    )))
    FROM json_each(training_plans.phases_json)
)
WHERE json_valid(phases_json)
  AND json_type(phases_json) = 'array'
  AND EXISTS (SELECT 1 FROM json_each(training_plans.phases_json) WHERE json_extract(value, '$.phase') IS NOT NULL);
