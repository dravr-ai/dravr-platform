-- ABOUTME: PostgreSQL sync state + soft-delete columns on health tables
-- ABOUTME: Mirrors migrations/20260327000001_sync_state.sql with PG-native types
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

CREATE TABLE IF NOT EXISTS sync_state (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    data_type TEXT NOT NULL,
    cursor_value TEXT,
    last_sync_at TIMESTAMPTZ,
    last_sync_status TEXT NOT NULL DEFAULT 'pending',
    records_synced BIGINT DEFAULT 0,
    error_message TEXT,
    retry_count INTEGER DEFAULT 0,
    next_retry_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(user_id, tenant_id, provider, data_type)
);
CREATE INDEX IF NOT EXISTS idx_sync_state_user_tenant ON sync_state(user_id, tenant_id);
CREATE INDEX IF NOT EXISTS idx_sync_state_next_retry ON sync_state(next_retry_at);

-- Add soft-delete support to health tables
ALTER TABLE sleep_sessions ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;
ALTER TABLE recovery_metrics ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;
ALTER TABLE health_snapshots ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;
ALTER TABLE data_point_series ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;
