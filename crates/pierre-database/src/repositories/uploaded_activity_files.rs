// ABOUTME: UploadedActivityFileRepository trait plus the one shared implementation both backends emit
// ABOUTME: What names each .fit file an athlete uploaded — owner, hash, size, origin; its bytes live in the blob store
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Uploaded activity files.
//!
//! An athlete can upload the `.fit` file of a completed workout (carnet#818).
//! Each session in it becomes a cached activity under the provider key
//! `upload`. The file itself is kept in the blob store by the originals store
//! (`pierre_providers::originals`); a row here names it: its owner, the
//! SHA-256 of its bytes, its size, its origin and when it came in. A file is
//! addressed by its hash, so the same file uploaded twice is one row, and the
//! row is what scopes a read of the file to the athlete who uploaded it.
//!
//! The table holds no bytes: the column that kept them before the blob store
//! did was drained into it and dropped (Athlete Data Consolidation, S12;
//! carnet#1309).
//!
//! Every statement is scoped to the tenant and the user. `$n` placeholders
//! throughout: sqlx accepts them on `SQLite` as well as Postgres, and every
//! id column is `TEXT` on both engines.

use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use uuid::Uuid;

/// The origin of a file the athlete uploaded from the app.
pub const UPLOAD_ORIGIN: &str = "upload";

/// The metadata of the activity files athletes uploaded.
#[async_trait]
pub trait UploadedActivityFileRepository: Send + Sync {
    /// Record one uploaded file whose bytes the blob store now holds. A file
    /// the user already holds in this tenant (the same hash) is left as it
    /// is.
    ///
    /// Returns `true` when the row was written, `false` when it was already
    /// held.
    ///
    /// # Errors
    /// Returns a database error when the write fails.
    async fn record_uploaded_file(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        file_sha256: &str,
        byte_size: u64,
        origin: &str,
    ) -> AppResult<bool>;

    /// Whether the user holds the file `file_sha256` in this tenant.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn holds_uploaded_file(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        file_sha256: &str,
    ) -> AppResult<bool>;

    /// Whether any activity the file `file_sha256` became is still cached for
    /// the user in this tenant: an `upload` row whose id is the hash followed
    /// by a session index (`<sha>-<index>`).
    ///
    /// Read from the ids the cache already holds, so the stored file is not
    /// decoded again to learn how many sessions it named.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    async fn file_backs_cached_activity(
        &self,
        tenant_id: &TenantId,
        user_id: Uuid,
        file_sha256: &str,
    ) -> AppResult<bool>;

    /// Delete one uploaded file's row, once the athlete has deleted every
    /// activity it became and its bytes are gone from the blob store.
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

/// Record a file once: a second upload of the same bytes writes nothing.
pub(crate) const RECORD_UPLOADED_FILE_SQL: &str = r"
    INSERT INTO uploaded_activity_files (
        tenant_id, user_id, file_sha256, byte_size, origin, uploaded_at
    )
    VALUES ($1, $2, $3, $4, $5, $6)
    ON CONFLICT (tenant_id, user_id, file_sha256) DO NOTHING";

/// Whether the owner holds the file.
pub(crate) const HOLDS_UPLOADED_FILE_SQL: &str = r"
    SELECT 1 AS held
    FROM uploaded_activity_files
    WHERE tenant_id = $1 AND user_id = $2 AND file_sha256 = $3";

/// Whether any cached `upload` activity of the owner's carries an id
/// starting with `$3`, a LIKE prefix whose wildcards are escaped. `SQLite`'s
/// LIKE ignores ASCII case, which changes nothing here: every id is a
/// lowercase hex hash, a dash and a decimal index.
pub(crate) const FILE_BACKS_CACHED_ACTIVITY_SQL: &str = r"
    SELECT 1 AS held
    FROM cached_activities
    WHERE user_id = $1 AND tenant_id = $2 AND provider = 'upload'
      AND activity_id LIKE $3 ESCAPE '\'
    LIMIT 1";

/// The LIKE pattern matching every activity id the file `file_sha256` names.
pub(crate) fn file_activity_id_pattern(file_sha256: &str) -> String {
    let mut pattern = String::with_capacity(file_sha256.len() + 2);
    for c in file_sha256.chars() {
        if matches!(c, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push_str("-%");
    pattern
}

/// Delete one file's row, by its owner and hash.
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
            async fn record_uploaded_file(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                file_sha256: &str,
                byte_size: u64,
                origin: &str,
            ) -> AppResult<bool> {
                let written = sqlx::query(RECORD_UPLOADED_FILE_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(file_sha256)
                    .bind(i64::try_from(byte_size).unwrap_or(i64::MAX))
                    .bind(origin)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("record_uploaded_file: {e}")))?;
                Ok(written.rows_affected() > 0)
            }

            async fn holds_uploaded_file(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                file_sha256: &str,
            ) -> AppResult<bool> {
                let row = sqlx::query(HOLDS_UPLOADED_FILE_SQL)
                    .bind(tenant_id.to_string())
                    .bind(user_id.to_string())
                    .bind(file_sha256)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("holds_uploaded_file: {e}")))?;
                Ok(row.is_some())
            }

            async fn file_backs_cached_activity(
                &self,
                tenant_id: &TenantId,
                user_id: Uuid,
                file_sha256: &str,
            ) -> AppResult<bool> {
                let row = sqlx::query(FILE_BACKS_CACHED_ACTIVITY_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id.to_string())
                    .bind(file_activity_id_pattern(file_sha256))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("file_backs_cached_activity: {e}")))?;
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

#[cfg(test)]
mod tests {
    use super::file_activity_id_pattern;

    #[test]
    fn the_pattern_is_the_hash_a_dash_and_any_index() {
        assert_eq!(file_activity_id_pattern("ab12"), "ab12-%");
    }

    #[test]
    fn like_wildcards_in_the_hash_are_matched_literally() {
        assert_eq!(file_activity_id_pattern("a_b%c\\d"), "a\\_b\\%c\\\\d-%");
    }
}
