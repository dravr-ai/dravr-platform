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
    /// When the last sync completed. `DateTime` so the PG backend binds a
    /// native TIMESTAMPTZ (a TEXT bind is rejected); `SQLite` stores RFC 3339 TEXT
    /// via sqlx's chrono encoding.
    pub last_sync_at: Option<DateTime<Utc>>,
    /// Current sync status (pending, completed, failed)
    pub last_sync_status: String,
    /// Total records synced in the last batch
    pub records_synced: i64,
    /// Error message from the last failed sync
    pub error_message: Option<String>,
    /// Number of consecutive retry attempts. `i32` so the PG backend decodes
    /// the INTEGER (INT4) column natively — sqlx's strict PG decode rejects an
    /// `i64` read of INT4 (and `Row::get` panics on it); `SQLite` INTEGER
    /// decodes either width.
    pub retry_count: i32,
    /// When to attempt the next retry
    pub next_retry_at: Option<DateTime<Utc>>,
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
    /// Stored credential kind from `user_oauth_tokens.token_type` —
    /// `"Bearer"` for OAuth exchanges, `"session"` for sciotte browser
    /// sessions (a plain string column, not a UUID, so `String` is fine
    /// here). Lets sync callers skip users whose stored credential the
    /// provider's sync implementation cannot use.
    pub token_type: String,
}

/// Where a stored wellness row came from: the provider it was stored under
/// and the data source that synced it.
///
/// Enough to govern a row by its provider terms without reading its
/// readings, for a caller that only asks whether any row is there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WellnessRowOrigin {
    /// The provider the row was stored under (`whoop`, `garmin`, ...).
    pub provider: String,
    /// The data source that synced the row; empty when none was recorded.
    pub data_source_id: String,
}

/// How a day's recovery or body write lands on the row its key already
/// names (Athlete Data Consolidation design §4.2).
///
/// Whichever mode, a write onto a soft-deleted row brings it back with only
/// the readings the write carries: nothing stored before the delete returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WellnessWriteMode {
    /// The whole day as its source holds it now (a sync of the day): every
    /// reading replaces the stored one, so a reading the source no longer
    /// holds is cleared. The row is filed under the write's data source.
    FullDay,
    /// One of the summaries a day arrives in (a push's dailies, its HRV, its
    /// body composition): a reading the summary carries replaces the stored
    /// one, a reading it leaves `None` is not part of the summary and stays.
    /// The ingresses that write this way never retract a reading.
    PartialSummary,
}

/// Sleep session persistence repository
#[async_trait]
pub trait SleepRepository: Send + Sync {
    /// Write a night's sleep received at `synced_at`, whole, filed under the
    /// session's data source — unless the stored night was received later,
    /// in which case it stands and `None` is returned. Returns the stored
    /// row's id otherwise (the existing row's on the update path).
    async fn upsert_sleep_session(
        &self,
        tenant_id: &TenantId,
        session: &StoredSleepSession,
        synced_at: DateTime<Utc>,
    ) -> AppResult<Option<String>>;

    /// Clear the provider-scored figure (`sleep_score`) from every night
    /// `user_id` holds under `provider`: the terms of that provider forbid
    /// keeping it. Returns how many nights held one.
    async fn clear_provider_sleep_scores(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
    ) -> AppResult<u64>;
    /// Get sleep sessions for a user within a date range
    async fn get_sleep_sessions(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<Vec<StoredSleepSession>>;
    /// The distinct origins of the sleep sessions starting inside the window,
    /// without reading a session.
    async fn sleep_origins(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<Vec<WellnessRowOrigin>>;
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
    /// Write a day's recovery readings received at `synced_at`, as `mode`
    /// says, filed under `metrics.data_source_id`. The row keeps the later
    /// of its stored and new `synced_at`. Returns the stored row's id.
    async fn upsert_recovery_metrics(
        &self,
        tenant_id: &TenantId,
        metrics: &StoredRecoveryMetrics,
        mode: WellnessWriteMode,
        synced_at: DateTime<Utc>,
    ) -> AppResult<String>;

    /// Clear every provider-scored composite (recovery and readiness score,
    /// stress, Body Battery, day strain) from every day `user_id` holds under
    /// `provider`: the terms of that provider forbid keeping them. Returns how
    /// many days held one.
    async fn clear_provider_recovery_scores(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        provider: &str,
    ) -> AppResult<u64>;
    /// Get recovery metrics for a user within a date range
    async fn get_recovery_metrics(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<Vec<StoredRecoveryMetrics>>;
    /// The distinct origins of the recovery readings dated inside the window,
    /// without reading a reading.
    async fn recovery_origins(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<Vec<WellnessRowOrigin>>;
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
    /// Insert or update a health snapshot for a date.
    ///
    /// A row's identity is the natural key `(user, tenant, provider, date)`; the
    /// returned `id` is a **store-assigned surrogate**, not the provider's id.
    /// `snapshot.id` is ignored on insert — provider ids are not guaranteed to
    /// vary with the natural key (WHOOP issues one constant `whoop-body-{user}`
    /// for every date), and reusing one collides with the primary key on the
    /// insert path before the conflict arbiter is consulted. On the update path
    /// the pre-existing row's id is returned unchanged.
    ///
    /// The write lands as `mode` says, received at `synced_at`; the row keeps
    /// the later of its stored and new `synced_at`.
    async fn upsert_health_snapshot(
        &self,
        tenant_id: &TenantId,
        snapshot: &StoredHealthMetrics,
        mode: WellnessWriteMode,
        synced_at: DateTime<Utc>,
    ) -> AppResult<String>;
    /// Get health snapshots for a user within a date range
    async fn get_health_snapshots(
        &self,
        user_id: Uuid,
        tenant_id: &TenantId,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<Vec<StoredHealthMetrics>>;
    /// Delete a single health snapshot by id with the requested deletion mode.
    ///
    /// `soft = true` sets `deleted_at = now()` and reads filter the row out;
    /// `soft = false` issues a hard `DELETE`.
    /// Returns `Ok(false)` if no matching row exists for the tenant.
    ///
    /// The `id` is the store-assigned surrogate returned by
    /// [`HealthSnapshotRepository::upsert_health_snapshot`], never a provider
    /// id — a caller holding only the provider's own identifier cannot resolve
    /// a row here.
    async fn delete_health_snapshot_by_id(
        &self,
        tenant_id: &TenantId,
        id: &str,
        soft: bool,
    ) -> AppResult<bool>;

    /// Look up the owning tenant for a health snapshot id.
    ///
    /// See [`SleepRepository::find_sleep_session_tenant`] for the rationale.
    /// Takes the store-assigned surrogate id, not a provider id.
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

    /// Delete every cursor `user_id` holds for `provider` under `tenant_id`,
    /// so the next sync reads the provider from its newest page again.
    /// Returns how many cursors were deleted.
    async fn reset_sync_cursors(
        &self,
        user_id: &str,
        tenant_id: &TenantId,
        provider: &str,
    ) -> AppResult<u64>;
}
