// ABOUTME: UploadedActivityFileRepository trait plus the one shared implementation both backends emit
// ABOUTME: The .fit files athletes uploaded, keyed by (tenant, user, SHA-256), read back only by their owner
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Uploaded activity files.
//!
//! An athlete can upload the `.fit` file of a completed workout (carnet#818).
//! Each session in it becomes a cached activity under the provider key
//! `upload`; the file is kept here, so the series, laps and route its
//! activities carry are read from it on demand. A file is addressed by the
//! SHA-256 of its bytes, so the same file uploaded twice is one row.
//!
//! Every statement is scoped to the tenant and the user: a file is only ever
//! read back by the athlete who uploaded it, in the tenant they uploaded it
//! in. `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
//! Postgres, and every id column is `TEXT` on both engines.

use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use uuid::Uuid;

/// Persistence for the activity files athletes uploaded.
#[async_trait]
pub trait UploadedActivityFileRepository: Send + Sync {
    /// Keep one uploaded file. A file the user already holds in this tenant
    /// (the same hash) is left as it is.
    ///
    /// Returns `true` when the row was written, `false` when it was already
    /// held.
    ///
    /// # Errors
    /// Returns a database error when the write fails.
    async fn store_uploaded_file(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        file_sha256: &str,
        bytes: &[u8],
    ) -> AppResult<bool>;

    /// The bytes of one uploaded file, or `None` when the user holds no file
    /// with that hash in this tenant.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn get_uploaded_file(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        file_sha256: &str,
    ) -> AppResult<Option<Vec<u8>>>;

    /// Whether the user holds any uploaded file in this tenant.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn has_uploaded_files(&self, tenant_id: &TenantId, user_id: Uuid) -> AppResult<bool>;

    /// Delete one uploaded file, once the athlete has deleted every activity
    /// it became.
    ///
    /// Returns `true` when a row was removed, `false` when the user held no
    /// file with that hash in this tenant.
    ///
    /// # Errors
    /// Returns a database error when the delete fails.
    async fn delete_uploaded_file(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        file_sha256: &str,
    ) -> AppResult<bool>;
}

/// Keep a file once: a second upload of the same bytes writes nothing.
pub(crate) const STORE_UPLOADED_FILE_SQL: &str = r"
    INSERT INTO uploaded_activity_files (
        tenant_id, user_id, file_sha256, byte_size, file_bytes, uploaded_at
    )
    VALUES ($1, $2, $3, $4, $5, $6)
    ON CONFLICT (tenant_id, user_id, file_sha256) DO NOTHING";

/// One file's bytes, by its owner and hash.
pub(crate) const GET_UPLOADED_FILE_SQL: &str = r"
    SELECT file_bytes
    FROM uploaded_activity_files
    WHERE tenant_id = $1 AND user_id = $2 AND file_sha256 = $3";

/// Whether the owner holds any file.
pub(crate) const HAS_UPLOADED_FILES_SQL: &str = r"
    SELECT 1 AS held
    FROM uploaded_activity_files
    WHERE tenant_id = $1 AND user_id = $2
    LIMIT 1";

/// Delete one file, by its owner and hash.
pub(crate) const DELETE_UPLOADED_FILE_SQL: &str = r"
    DELETE FROM uploaded_activity_files
    WHERE tenant_id = $1 AND user_id = $2 AND file_sha256 = $3";

/// Emit the whole [`UploadedActivityFileRepository`] implementation for one
/// backend type. The body is written once here; each backend's shell invokes
/// it with its own type, and sqlx resolves the driver from `self.pool()` per
/// expansion. The body names its consts and types unqualified, so the
/// invoking shell must `use` every one of them.
macro_rules! impl_uploaded_activity_file_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl UploadedActivityFileRepository for $ty {
            async fn store_uploaded_file(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                file_sha256: &str,
                bytes: &[u8],
            ) -> AppResult<bool> {
                let written = sqlx::query(STORE_UPLOADED_FILE_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(file_sha256)
                    .bind(i64::try_from(bytes.len()).unwrap_or(i64::MAX))
                    .bind(bytes)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("store_uploaded_file: {e}")))?;
                Ok(written.rows_affected() > 0)
            }

            async fn get_uploaded_file(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                file_sha256: &str,
            ) -> AppResult<Option<Vec<u8>>> {
                let row = sqlx::query(GET_UPLOADED_FILE_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(file_sha256)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("get_uploaded_file: {e}")))?;
                row.map(|row| {
                    row.try_get::<Vec<u8>, _>("file_bytes")
                        .map_err(|e| AppError::database(format!("uploaded file bytes: {e}")))
                })
                .transpose()
            }

            async fn has_uploaded_files(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
            ) -> AppResult<bool> {
                let row = sqlx::query(HAS_UPLOADED_FILES_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("has_uploaded_files: {e}")))?;
                Ok(row.is_some())
            }

            async fn delete_uploaded_file(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                file_sha256: &str,
            ) -> AppResult<bool> {
                let deleted = sqlx::query(DELETE_UPLOADED_FILE_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(file_sha256)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("delete_uploaded_file: {e}")))?;
                Ok(deleted.rows_affected() > 0)
            }
        }
    };
}
pub(crate) use impl_uploaded_activity_file_repository;
