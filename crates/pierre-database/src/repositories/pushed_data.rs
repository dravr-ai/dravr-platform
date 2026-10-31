// ABOUTME: The transient copy of an activity file a provider pushes once, kept until its samples are stored
// ABOUTME: The SQL is written once here and both backends emit their impl from it; pushed rows go through the one writers

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Pushed activity files.
//!
//! A provider that pushes its data (the Garmin Connect Developer Program,
//! carnet#1155) offers an activity's file once. The copy is kept from its
//! download until the samples it carries are stored
//! ([`PushedDataRepository::keep_file`]), so a write that fails after the
//! download, or an instance that dies, does not lose a file the provider
//! will not serve again. That copy is a transient retry buffer, dropped once
//! the samples land and swept after 24 hours whatever became of it; it is
//! never a store of original files.
//!
//! Nothing else a push brings is written here. Its activities go to the one
//! `cached_activities` writer
//! ([`crate::repositories::ActivityCacheRepository`]), its samples to the one
//! sample store ([`crate::repositories::ActivityStreamRepository`]), and its
//! wellness to the wellness repositories in their partial-summary mode
//! ([`crate::repositories::WellnessWriteMode::PartialSummary`]), each with
//! the latest-receipt rule a push needs: batches run out of order, and the
//! one that lands last stands.
//!
//! `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
//! Postgres. Instants bind as `DateTime<Utc>`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use uuid::Uuid;

/// The kept copies of the activity files a provider pushes.
#[async_trait]
pub trait PushedDataRepository: Send + Sync {
    /// Keep a downloaded activity file until its samples are stored,
    /// replacing any copy kept before.
    ///
    /// # Errors
    /// Returns a database error when the write fails.
    async fn keep_file(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        provider: &str,
        activity_id: &str,
        bytes: &[u8],
        fetched_at: DateTime<Utc>,
    ) -> AppResult<()>;

    /// The kept copy of an activity's file, or `None` when none is kept.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn kept_file(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        provider: &str,
        activity_id: &str,
    ) -> AppResult<Option<Vec<u8>>>;

    /// Drop the kept copy of an activity's file: its samples are stored, or
    /// it is not a readable activity.
    ///
    /// # Errors
    /// Returns a database error when the delete fails.
    async fn drop_file(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        provider: &str,
        activity_id: &str,
    ) -> AppResult<()>;

    /// Delete every kept file downloaded before `cutoff`, whoever it
    /// belongs to: one still kept that long belongs to a batch that was
    /// given up on. Returns how many were deleted.
    ///
    /// # Errors
    /// Returns a database error when the delete fails.
    async fn drop_files_fetched_before(&self, cutoff: DateTime<Utc>) -> AppResult<u64>;
}

/// Keep, or replace, one activity's downloaded file.
pub(crate) const KEEP_PUSHED_FILE_SQL: &str = r"
    INSERT INTO pushed_activity_files (tenant_id, user_id, provider, activity_id, file_bytes, fetched_at)
    VALUES ($1, $2, $3, $4, $5, $6)
    ON CONFLICT (tenant_id, user_id, provider, activity_id) DO UPDATE SET
        file_bytes = EXCLUDED.file_bytes,
        fetched_at = EXCLUDED.fetched_at";

/// One activity's kept file, scoped by tenant, user and provider.
pub(crate) const KEPT_PUSHED_FILE_SQL: &str = r"
    SELECT file_bytes FROM pushed_activity_files
    WHERE tenant_id = $1 AND user_id = $2 AND provider = $3 AND activity_id = $4";

/// Drop one activity's kept file.
pub(crate) const DROP_PUSHED_FILE_SQL: &str = r"
    DELETE FROM pushed_activity_files
    WHERE tenant_id = $1 AND user_id = $2 AND provider = $3 AND activity_id = $4";

/// Drop every kept file downloaded before `$1`.
///
/// ISOLATION EXEMPTION: this delete spans every tenant on purpose. It serves
/// the push worker's sweep, a system job that drops the copies of batches
/// given up on wherever they are held; it reads nothing back.
pub(crate) const DROP_STALE_PUSHED_FILES_SQL: &str =
    "DELETE FROM pushed_activity_files WHERE fetched_at < $1";

/// Emit the whole [`PushedDataRepository`] implementation for one backend
/// type. The body is written once here; each backend's shell invokes it with
/// its own type, and sqlx resolves the driver from `self.pool()` per
/// expansion. The body names its consts, helpers and types unqualified, so
/// the invoking shell must `use` every one of them.
macro_rules! impl_pushed_data_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl PushedDataRepository for $ty {
            async fn keep_file(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                provider: &str,
                activity_id: &str,
                bytes: &[u8],
                fetched_at: DateTime<Utc>,
            ) -> AppResult<()> {
                sqlx::query(KEEP_PUSHED_FILE_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(provider)
                    .bind(activity_id)
                    .bind(bytes)
                    .bind(fetched_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("keep pushed file: {e}")))?;
                Ok(())
            }

            async fn kept_file(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                provider: &str,
                activity_id: &str,
            ) -> AppResult<Option<Vec<u8>>> {
                sqlx::query_scalar(KEPT_PUSHED_FILE_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(provider)
                    .bind(activity_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("read kept pushed file: {e}")))
            }

            async fn drop_file(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                provider: &str,
                activity_id: &str,
            ) -> AppResult<()> {
                sqlx::query(DROP_PUSHED_FILE_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(provider)
                    .bind(activity_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("drop pushed file: {e}")))?;
                Ok(())
            }

            async fn drop_files_fetched_before(&self, cutoff: DateTime<Utc>) -> AppResult<u64> {
                sqlx::query(DROP_STALE_PUSHED_FILES_SQL)
                    .bind(cutoff)
                    .execute(self.pool())
                    .await
                    .map(|done| done.rows_affected())
                    .map_err(|e| AppError::database(format!("drop stale pushed files: {e}")))
            }
        }
    };
}
pub(crate) use impl_pushed_data_repository;
