-- ABOUTME: Create push notification tables for device tokens, preferences, notifications, and scheduling
-- ABOUTME: Supports multi-tenant notification delivery via Expo Push Service with per-user preference controls
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

-- ════════════════════════════════════════════════════════════════
-- Device tokens for Expo push notifications
-- ════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS device_tokens (
    id                  TEXT    NOT NULL PRIMARY KEY,
    user_id             TEXT    NOT NULL,
    tenant_id           TEXT    NOT NULL,
    expo_push_token     TEXT    NOT NULL,
    platform            TEXT    NOT NULL CHECK (platform IN ('ios', 'android')),
    device_name         TEXT,
    active              INTEGER NOT NULL DEFAULT 1,
    created_at          TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at          TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(user_id, tenant_id, expo_push_token)
);

CREATE INDEX IF NOT EXISTS idx_device_tokens_user_tenant
    ON device_tokens(user_id, tenant_id);

CREATE INDEX IF NOT EXISTS idx_device_tokens_tenant
    ON device_tokens(tenant_id);

CREATE INDEX IF NOT EXISTS idx_device_tokens_active
    ON device_tokens(user_id, tenant_id, active) WHERE active = 1;

-- ════════════════════════════════════════════════════════════════
-- Notification preferences (one row per user + tenant + category)
-- ════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS notification_preferences (
    id                  TEXT    NOT NULL PRIMARY KEY,
    user_id             TEXT    NOT NULL,
    tenant_id           TEXT    NOT NULL,
    category            TEXT    NOT NULL CHECK (category IN ('training', 'recovery', 'social', 'coach', 'achievement', 'system', 'ai', 'reminders')),
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

CREATE INDEX IF NOT EXISTS idx_notification_preferences_user_tenant
    ON notification_preferences(user_id, tenant_id);

-- ════════════════════════════════════════════════════════════════
-- Notifications (the actual notification records)
-- ════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS notifications (
    id                  TEXT    NOT NULL PRIMARY KEY,
    user_id             TEXT    NOT NULL,
    tenant_id           TEXT    NOT NULL,
    category            TEXT    NOT NULL,
    notification_type   TEXT    NOT NULL,
    title               TEXT    NOT NULL,
    body                TEXT    NOT NULL,
    data                TEXT,   -- JSON object for deep-link routing and action payloads
    image_url           TEXT,
    read_at             TEXT,
    delivered_at        TEXT,
    opened_at           TEXT,
    dismissed_at        TEXT,
    created_at          TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_notifications_user_tenant
    ON notifications(user_id, tenant_id);

CREATE INDEX IF NOT EXISTS idx_notifications_user_tenant_created
    ON notifications(user_id, tenant_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_notifications_user_tenant_category
    ON notifications(user_id, tenant_id, category);

CREATE INDEX IF NOT EXISTS idx_notifications_user_tenant_unread
    ON notifications(user_id, tenant_id, read_at) WHERE read_at IS NULL;

-- ════════════════════════════════════════════════════════════════
-- Scheduled notifications (cron-based recurring notifications)
-- ════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS scheduled_notifications (
    id                  TEXT    NOT NULL PRIMARY KEY,
    user_id             TEXT    NOT NULL,
    tenant_id           TEXT    NOT NULL,
    notification_type   TEXT    NOT NULL,
    schedule_cron       TEXT    NOT NULL,
    timezone            TEXT    NOT NULL DEFAULT 'UTC',
    next_fire_at        TEXT,
    enabled             INTEGER NOT NULL DEFAULT 1,
    last_fired_at       TEXT,
    created_at          TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_scheduled_notifications_user_tenant
    ON scheduled_notifications(user_id, tenant_id);

CREATE INDEX IF NOT EXISTS idx_scheduled_notifications_next_fire
    ON scheduled_notifications(next_fire_at, enabled) WHERE enabled = 1;
