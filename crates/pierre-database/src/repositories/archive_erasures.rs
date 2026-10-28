// ABOUTME: Repository trait, statements and shared body for the archive erasure outbox of deleted accounts
// ABOUTME: A row per deleted account whose imported history dravr-barrage still holds; written once, emitted per backend

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Deleted accounts whose imported history is still to erase (carnet#1218).
//!
//! An athlete's imported history lives in dravr-barrage: records and jobs in
//! its own schema or database file, original recording files in its blob
//! store, none of which the account delete's transaction reaches. The delete
//! writes the account's id here in that transaction (see
//! `clear_user_in_transaction`), so it is never lost between the commit and
//! the erasure; the sweep erases what barrage holds for the account, in every
//! tenant, and removes the row once barrage confirms, recording each failure
//! with the time of the next attempt.
//!
//! ISOLATION EXEMPTION: the table carries no `tenant_id`, since an account
//! spans every tenant, and names the account `erased_account_id` rather than
//! `user_id`: it outlives the account by design and cascades from nothing.

use std::fmt::Display;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};

/// One deleted account still to erase from barrage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingArchiveErasure {
    /// The deleted account's id.
    pub erased_account_id: String,
    /// Failed attempts so far.
    pub attempts: i64,
}

/// The archive erasure outbox.
#[async_trait]
pub trait ArchiveErasureRepository: Send + Sync {
    /// The rows due at `now`, oldest attempt first, at most `limit`.
    async fn due(&self, now: DateTime<Utc>, limit: i64) -> AppResult<Vec<PendingArchiveErasure>>;

    /// Remove the row of `erased_account_id`: barrage holds nothing of the
    /// account any more. Returns whether a row was removed.
    async fn complete(&self, erased_account_id: &str) -> AppResult<bool>;

    /// Record a failed attempt: bump the count, keep `error`, and hold the
    /// row until `next_attempt_at`.
    async fn record_failure(
        &self,
        erased_account_id: &str,
        error: &str,
        next_attempt_at: DateTime<Utc>,
    ) -> AppResult<()>;
}

/// Queue the account being deleted, inside the account delete's transaction.
/// `$1` is the account id as text, `$2` now. An account already queued stays
/// as it is.
pub(crate) const QUEUE_ARCHIVE_ERASURE_SQL: &str = r"
            INSERT INTO archive_erasures
                (erased_account_id, attempts, next_attempt_at, last_error, created_at)
            VALUES ($1, 0, $2, NULL, $2)
            ON CONFLICT (erased_account_id) DO NOTHING
            ";

/// The rows due at `$1`, oldest first, capped at `$2`.
pub(crate) const DUE_ARCHIVE_ERASURES_SQL: &str = r"
            SELECT erased_account_id, attempts
            FROM archive_erasures
            WHERE next_attempt_at <= $1
            ORDER BY next_attempt_at
            LIMIT $2
            ";

/// Remove the row of an account barrage no longer holds anything of.
pub(crate) const COMPLETE_ARCHIVE_ERASURE_SQL: &str =
    "DELETE FROM archive_erasures WHERE erased_account_id = $1";

/// Count a failed attempt and hold the row until the next one.
pub(crate) const RECORD_ARCHIVE_ERASURE_FAILURE_SQL: &str = r"
            UPDATE archive_erasures
            SET attempts = attempts + 1, last_error = $2, next_attempt_at = $3
            WHERE erased_account_id = $1
            ";

/// The error for a column of this table that would not decode.
pub(crate) fn archive_erasure_column_error(name: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to read archive_erasures.{name}: {e}"))
}

/// Emit the whole [`ArchiveErasureRepository`] implementation for one backend
/// type; sqlx resolves the driver from `self.pool()`.
macro_rules! impl_archive_erasure_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl ArchiveErasureRepository for $ty {
            async fn due(
                &self,
                now: DateTime<Utc>,
                limit: i64,
            ) -> AppResult<Vec<PendingArchiveErasure>> {
                let rows = sqlx::query(DUE_ARCHIVE_ERASURES_SQL)
                    .bind(now)
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list pending archive erasures: {e}"))
                    })?;
                rows.iter()
                    .map(|row| {
                        Ok(PendingArchiveErasure {
                            erased_account_id: row.try_get("erased_account_id").map_err(|e| {
                                archive_erasure_column_error("erased_account_id", e)
                            })?,
                            attempts: row
                                .try_get("attempts")
                                .map_err(|e| archive_erasure_column_error("attempts", e))?,
                        })
                    })
                    .collect()
            }

            async fn complete(&self, erased_account_id: &str) -> AppResult<bool> {
                let done = sqlx::query(COMPLETE_ARCHIVE_ERASURE_SQL)
                    .bind(erased_account_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to remove an archive erasure: {e}"))
                    })?;
                Ok(done.rows_affected() > 0)
            }

            async fn record_failure(
                &self,
                erased_account_id: &str,
                error: &str,
                next_attempt_at: DateTime<Utc>,
            ) -> AppResult<()> {
                sqlx::query(RECORD_ARCHIVE_ERASURE_FAILURE_SQL)
                    .bind(erased_account_id)
                    .bind(error)
                    .bind(next_attempt_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to record an archive erasure failure: {e}"
                        ))
                    })?;
                Ok(())
            }
        }
    };
}
pub(crate) use impl_archive_erasure_repository;
