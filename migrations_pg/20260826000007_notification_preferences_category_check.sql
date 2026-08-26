-- ABOUTME: Re-creates the notification_preferences category CHECK to match NotificationCategory::all().
-- ABOUTME: The constraint written in 20260311000007 still admitted the retired 'social' string.
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The category vocabulary is dravr-commere's NotificationCategory::all():
-- training, recovery, coach, achievement, system, ai, reminders. The CHECK
-- from 20260311000007 predates the Chat-First Cutover and still lists
-- 'social' — the category commere 0.2.0 stopped parsing and 20260826000006
-- deleted the rows of — so the schema admitted a string the application
-- refuses. PostgreSQL named that inline column constraint
-- notification_preferences_category_check; it is replaced under the same
-- name so the table keeps exactly one category CHECK and a violation reports
-- it by name on both backends. Every stored row satisfies the new constraint:
-- 'social' rows are already gone and 'group' was never admitted.
ALTER TABLE notification_preferences
    DROP CONSTRAINT notification_preferences_category_check;
ALTER TABLE notification_preferences
    ADD CONSTRAINT notification_preferences_category_check
    CHECK (category IN ('training', 'recovery', 'coach', 'achievement', 'system', 'ai', 'reminders'));
