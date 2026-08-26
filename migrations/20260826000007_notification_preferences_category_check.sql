-- ABOUTME: Rebuilds notification_preferences so its category CHECK matches NotificationCategory::all().
-- ABOUTME: The constraint written in 20260310000001 still admitted the retired 'social' string; SQLite cannot ALTER a CHECK.
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- The category vocabulary is dravr-commere's NotificationCategory::all():
-- training, recovery, coach, achievement, system, ai, reminders. The CHECK
-- from 20260310000001 predates the Chat-First Cutover and still lists
-- 'social' — the category commere 0.2.0 stopped parsing and 20260826000006
-- deleted the rows of — so the schema admitted a string the application
-- refuses. SQLite has no ALTER for a CHECK constraint: the table is copied
-- under the corrected constraint, the original dropped, the copy renamed,
-- and the index recreated. The constraint is named so a violation reports it
-- by name on both backends. sqlx runs the migration in one transaction and
-- no other table references this one by foreign key, so no PRAGMA is needed.
-- Every row survives the copy: 'social' rows are already gone and 'group'
-- was never admitted, so nothing the old constraint stored fails the new one.
CREATE TABLE IF NOT EXISTS notification_preferences_new (
    id                  TEXT    NOT NULL PRIMARY KEY,
    user_id             TEXT    NOT NULL,
    tenant_id           TEXT    NOT NULL,
    category            TEXT    NOT NULL
                        CONSTRAINT notification_preferences_category_check
                        CHECK (category IN ('training', 'recovery', 'coach', 'achievement', 'system', 'ai', 'reminders')),
    enabled             INTEGER NOT NULL DEFAULT 1,
    sub_preferences     TEXT,   -- JSON object for granular per-type toggles
    quiet_hours_start   TEXT,   -- HH:MM format
    quiet_hours_end     TEXT,   -- HH:MM format
    timezone            TEXT,
    max_per_day         INTEGER,
    created_at          TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at          TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(user_id, tenant_id, category)
);
INSERT INTO notification_preferences_new (id, user_id, tenant_id, category, enabled,
    sub_preferences, quiet_hours_start, quiet_hours_end, timezone, max_per_day,
    created_at, updated_at)
  SELECT id, user_id, tenant_id, category, enabled, sub_preferences,
    quiet_hours_start, quiet_hours_end, timezone, max_per_day, created_at, updated_at
  FROM notification_preferences;
DROP TABLE IF EXISTS notification_preferences;
ALTER TABLE notification_preferences_new RENAME TO notification_preferences;

CREATE INDEX IF NOT EXISTS idx_notification_preferences_user_tenant
    ON notification_preferences(user_id, tenant_id);
