-- ABOUTME: Clears the WHOOP proprietary scores already stored and recomputes WHOOP sleep efficiency in-house
-- ABOUTME: Recovery %, strain, sleep performance and WHOOP's own efficiency are WHOOP's calculations (API Terms §4)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Health sync stored WHOOP's own scores as it received them: recovery % in
-- recovery_metrics.recovery_score, day strain in recovery_metrics.training_load,
-- sleep performance in sleep_sessions.sleep_score, WHOOP's sleep efficiency in
-- sleep_sessions.sleep_efficiency, and workout strain as a cached activity's
-- training_stress_score. Those are WHOOP's calculations, which only WHOOP can
-- authorize storing, and ingestion now drops them. This clears the copies
-- already written. Provenance is each table's provider column, which every row
-- touched here carries.

UPDATE recovery_metrics
SET recovery_score = NULL,
    readiness_score = NULL,
    stress_level = NULL,
    body_battery = NULL,
    training_load = NULL,
    sleep_score = NULL
WHERE provider = 'whoop';

-- Dravr's own sleep efficiency is the share of the session not spent awake:
--   time_in_bed = end_time - start_time, in whole seconds
--   efficiency  = (time_in_bed - awake_seconds) / time_in_bed * 100
-- the formula ingestion applies. It needs awake_seconds, which rows written
-- before that column existed lack; those, and empty or inconsistent sessions,
-- get 0, which the NOT NULL column reads back as absent.
UPDATE sleep_sessions
SET sleep_score = NULL,
    sleep_efficiency = CASE
        WHEN awake_seconds IS NOT NULL
         AND FLOOR(EXTRACT(EPOCH FROM (end_time - start_time))) > 0
         AND awake_seconds <= FLOOR(EXTRACT(EPOCH FROM (end_time - start_time)))
        THEN (FLOOR(EXTRACT(EPOCH FROM (end_time - start_time))) - awake_seconds) * 100.0
             / FLOOR(EXTRACT(EPOCH FROM (end_time - start_time)))
        ELSE 0
    END
WHERE provider = 'whoop';

UPDATE cached_activities
SET data_json = (data_json::jsonb - 'training_stress_score')::text
WHERE provider = 'whoop'
  AND (data_json::jsonb -> 'training_stress_score') IS NOT NULL;
