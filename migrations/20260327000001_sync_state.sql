-- Sync state tracking for CDC-based incremental sync
-- Tracks cursor position per user/provider/data_type
--
-- SPDX-License-Identifier: MIT OR Apache-2.0
-- Copyright (c) 2026 dravr.ai

CREATE TABLE IF NOT EXISTS sync_state (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    data_type TEXT NOT NULL,
    cursor_value TEXT,
    last_sync_at TEXT,
    last_sync_status TEXT NOT NULL DEFAULT 'pending',
    records_synced INTEGER DEFAULT 0,
    error_message TEXT,
    retry_count INTEGER DEFAULT 0,
    next_retry_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(user_id, tenant_id, provider, data_type)
);
CREATE INDEX IF NOT EXISTS idx_sync_state_user_tenant ON sync_state(user_id, tenant_id);
CREATE INDEX IF NOT EXISTS idx_sync_state_next_retry ON sync_state(next_retry_at);

-- Add soft-delete support to health tables
ALTER TABLE sleep_sessions ADD COLUMN deleted_at TEXT;
ALTER TABLE recovery_metrics ADD COLUMN deleted_at TEXT;
ALTER TABLE health_snapshots ADD COLUMN deleted_at TEXT;
ALTER TABLE data_point_series ADD COLUMN deleted_at TEXT;
