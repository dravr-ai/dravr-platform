// ABOUTME: Repository trait definitions for the health/sleep/recovery/time-series persistence + sync cursors domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;

use pierre_core::models::{StoredHealthMetrics, StoredRecoveryMetrics, StoredSleepSession};
use pierre_core::models::{TenantId, UserId};
use uuid::Uuid;

use serde::{Deserialize, Serialize};

/// Database row for `sync_state` table
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncCursorRow {
    /// Unique identifier for this sync state record
    pub id: String,
    /// User who owns this sync cursor
    pub user_id: String,
    /// Tenant context for multi-tenant isolation
    pub tenant_id: String,
    /// Provider being synced (e.g., "whoop", "garmin")
    pub provider: String,
    /// Data type being synced (e.g., "sleep", "recovery", "health")
    pub data_type: String,
    /// Provider-specific cursor token for pagination
    pub cursor_value: Option<String>,
    /// When the last sync completed (RFC 3339)
    pub last_sync_at: Option<String>,
    /// Current sync status (pending, completed, failed)
    pub last_sync_status: String,
    /// Total records synced in the last batch
    pub records_synced: i64,
    /// Error message from the last failed sync
    pub error_message: Option<String>,
    /// Number of consecutive retry attempts
    pub retry_count: i64,
    /// When to attempt the next retry (RFC 3339)
    pub next_retry_at: Option<String>,
}

/// Connected user info for sync scheduling.
///
/// Fields use the [`UserId`] and [`TenantId`] newtypes (defined in
/// `pierre-core`) so the `SQLite` (TEXT) vs `PostgreSQL` (UUID) column-type split
/// is handled by the sqlx `Decode` impl, not by ad-hoc `r.get::<String, _>`
/// calls in each backend's row mapper. This is the canonical pattern for new
/// row structs — adding more `pub field: String` columns that hold UUIDs
/// reintroduces the panic surface that this struct replaced.
#[derive(Debug, Clone)]
pub struct ConnectedUserRow {
    /// User identifier
    pub user_id: UserId,
    /// Tenant identifier
    pub tenant_id: TenantId,
}

/// Sleep session persistence repository
#[async_trait]
pub trait SleepRepository: Send + Sync {
    /// Insert or update a sleep session
    async fn upsert_sleep_session(
        &self,
        tenant_id: &TenantId,
        session: &StoredSleepSession,
    ) -> AppResult<String>;
    /// Get sleep sessions for a user within a date range
    async fn get_sleep_sessions(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<Vec<StoredSleepSession>>;
    /// Get the most recent sleep session for a user
    async fn get_latest_sleep_session(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
    ) -> AppResult<Option<StoredSleepSession>>;
    /// Delete sleep sessions for a user and provider
    async fn delete_sleep_sessions(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
    ) -> AppResult<u64>;
    /// Delete a single sleep session by id with the requested deletion mode.
    ///
    /// `soft = true` sets `deleted_at = now()` and the row stays for the
    /// tombstone window so subsequent syncs can detect tombstones; reads
    /// already filter out soft-deleted rows.
    /// `soft = false` issues a hard `DELETE` and removes the row.
    /// Returns `Ok(false)` if no matching row exists for the tenant.
    async fn delete_sleep_session_by_id(
        &self,
        tenant_id: &TenantId,
        id: &str,
        soft: bool,
    ) -> AppResult<bool>;

    /// Look up the owning tenant for a sleep session id.
    ///
    /// Discovery query used by adapters that receive an external id with
    /// no tenant context (notably the dravr-enforme sync pipeline). The
    /// caller then calls [`Self::delete_sleep_session_by_id`] with the
    /// resolved tenant so the data-access query stays tenant-scoped.
    /// Returns `Ok(None)` if no row exists for the id.
    async fn find_sleep_session_tenant(&self, id: &str) -> AppResult<Option<TenantId>>;
}

/// Recovery metrics persistence repository
#[async_trait]
pub trait RecoveryRepository: Send + Sync {
    /// Insert or update recovery metrics for a date
    async fn upsert_recovery_metrics(
        &self,
        tenant_id: &TenantId,
        metrics: &StoredRecoveryMetrics,
    ) -> AppResult<String>;
    /// Get recovery metrics for a user within a date range
    async fn get_recovery_metrics(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<Vec<StoredRecoveryMetrics>>;
    /// Get the most recent recovery metrics for a user
    async fn get_latest_recovery(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
    ) -> AppResult<Option<StoredRecoveryMetrics>>;
    /// Delete a single recovery metric by id with the requested deletion mode.
    ///
    /// `soft = true` sets `deleted_at = now()` and reads filter the row out;
    /// `soft = false` issues a hard `DELETE`.
    /// Returns `Ok(false)` if no matching row exists for the tenant.
    async fn delete_recovery_metric_by_id(
        &self,
        tenant_id: &TenantId,
        id: &str,
        soft: bool,
    ) -> AppResult<bool>;

    /// Look up the owning tenant for a recovery metric id.
    ///
    /// See [`SleepRepository::find_sleep_session_tenant`] for the rationale.
    async fn find_recovery_metric_tenant(&self, id: &str) -> AppResult<Option<TenantId>>;
}

/// Health snapshot persistence repository
#[async_trait]
pub trait HealthSnapshotRepository: Send + Sync {
    /// Insert or update a health snapshot for a date
    async fn upsert_health_snapshot(
        &self,
        tenant_id: &TenantId,
        snapshot: &StoredHealthMetrics,
    ) -> AppResult<String>;
    /// Get health snapshots for a user within a date range
    async fn get_health_snapshots(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<Vec<StoredHealthMetrics>>;
    /// Get the most recent health snapshot for a user
    async fn get_latest_health_snapshot(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
    ) -> AppResult<Option<StoredHealthMetrics>>;
    /// Delete a single health snapshot by id with the requested deletion mode.
    ///
    /// `soft = true` sets `deleted_at = now()` and reads filter the row out;
    /// `soft = false` issues a hard `DELETE`.
    /// Returns `Ok(false)` if no matching row exists for the tenant.
    async fn delete_health_snapshot_by_id(
        &self,
        tenant_id: &TenantId,
        id: &str,
        soft: bool,
    ) -> AppResult<bool>;

    /// Look up the owning tenant for a health snapshot id.
    ///
    /// See [`SleepRepository::find_sleep_session_tenant`] for the rationale.
    async fn find_health_snapshot_tenant(&self, id: &str) -> AppResult<Option<TenantId>>;
}

/// Sync cursor repository for CDC-based incremental sync tracking
#[async_trait]
pub trait SyncCursorRepository: Send + Sync {
    /// Get the sync cursor for a user+provider+`data_type`
    async fn get_sync_cursor(
        &self,
        user_id: &str,
        tenant_id: &TenantId,
        provider: &str,
        data_type: &str,
    ) -> AppResult<Option<SyncCursorRow>>;

    /// Upsert (insert or update) a sync cursor
    async fn upsert_sync_cursor(&self, cursor: &SyncCursorRow) -> AppResult<()>;

    /// List all connected users for a given provider
    async fn list_connected_provider_users(
        &self,
        provider: &str,
    ) -> AppResult<Vec<ConnectedUserRow>>;
}
