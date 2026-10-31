-- ABOUTME: Clears the recovery and readiness scores Body Battery filled on stored Garmin days
-- ABOUTME: One reading, one column: Body Battery stays in body_battery (Athlete Data Consolidation, DECISION 2)

-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- Both Garmin ingresses (the usr/pwd capture through dravr-enforme and the
-- Developer Program push) copied the day's Body Battery into
-- recovery_metrics.recovery_score and readiness_score as well as
-- body_battery. Neither writes it there any more; this clears the copies
-- already stored. Only a score equal to that day's Body Battery is cleared,
-- which every Garmin score is: Garmin rows carry no other recovery or
-- readiness score. The values are recoverable from body_battery itself.

UPDATE recovery_metrics
SET recovery_score = NULL,
    readiness_score = NULL
WHERE provider = 'garmin'
  AND body_battery IS NOT NULL
  AND (recovery_score IS NULL OR recovery_score = body_battery)
  AND (readiness_score IS NULL OR readiness_score = body_battery)
  AND (recovery_score IS NOT NULL OR readiness_score IS NOT NULL);
