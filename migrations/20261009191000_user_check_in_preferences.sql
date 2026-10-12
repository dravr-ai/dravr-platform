-- ABOUTME: How often the coach checks in with each athlete on its own initiative, as the athlete chose it (SQLite)
-- ABOUTME: Daily, three per week, weekly or off; no row is an athlete who never chose, and is never checked in on (carnet#831)
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- One row per athlete and tenant, written when they answer the onboarding
-- question or change the setting. The tenant is the one whose threads the
-- check-in lands in, so every read is keyed by tenant_id and user_id, and the
-- scheduler lists a tenant's subscribers by tenant_id.
--   cadence        'daily' | 'three_per_week' | 'weekly' | 'off'
--   updated_at_ms  epoch milliseconds of the last choice, identical on both engines
CREATE TABLE IF NOT EXISTS user_check_in_preferences (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    cadence TEXT NOT NULL CHECK (cadence IN ('daily', 'three_per_week', 'weekly', 'off')),
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY (tenant_id, user_id)
);

-- The scheduler's per-tick read: a tenant's athletes who chose a cadence.
CREATE INDEX IF NOT EXISTS idx_user_check_in_preferences_tenant_cadence
    ON user_check_in_preferences (tenant_id, cadence);
