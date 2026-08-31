-- ABOUTME: Backfill goal_data.goal_id with the row id for goals stored without one
-- ABOUTME: Progress tracking finds a goal by goal_data.goal_id, so every stored goal must carry it

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- create_goal embeds the generated row id into the stored JSON as goal_id, but
-- rows written before it did carry no goal_id, so track_progress could never
-- find them ("Goal not found" for every goal). Copy the row id into the JSON
-- for exactly those rows; rows already carrying a goal_id are left untouched,
-- which also makes the statement idempotent.
UPDATE goals
SET goal_data = jsonb_set(goal_data, '{goal_id}', to_jsonb(id))
WHERE jsonb_typeof(goal_data) = 'object'
  AND goal_data->>'goal_id' IS NULL;
