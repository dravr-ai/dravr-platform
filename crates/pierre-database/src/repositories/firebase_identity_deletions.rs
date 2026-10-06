// ABOUTME: Repository trait, statements and shared body for the Firebase identity deletion outbox
// ABOUTME: A row per Firebase uid still to delete at Google after its account went; written once, emitted per backend

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Firebase identities still to delete (carnet#798).
//!
//! The account delete writes the account's Firebase uid here in its own
//! transaction (see `delete_user_completely`), so the identity is never lost
//! between the commit and the call to Google. The post-commit attempt and the
//! sweep remove the row once Firebase confirms, and record each failure with
//! the time of the next attempt.
//!
//! The table carries neither `tenant_id` nor `user_id` by design: it outlives
//! the account and must not keep a pointer to it. Its key is the Firebase uid.

use std::fmt::Display;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};

/// One Firebase identity still to delete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingFirebaseDeletion {
    /// The Firebase Authentication uid.
    pub firebase_uid: String,
    /// The Firebase project the uid belongs to.
    pub firebase_project: String,
    /// Failed attempts so far.
    pub attempts: i64,
    /// When the sweep may try again.
    pub next_attempt_at: DateTime<Utc>,
    /// The last attempt's failure.
    pub last_error: Option<String>,
}

/// The Firebase identity deletion outbox.
#[async_trait]
pub trait FirebaseIdentityDeletionRepository: Send + Sync {
    /// The rows due at `now`, oldest attempt first, at most `limit`; a uid a
    /// live account holds again is never due.
    async fn due(&self, now: DateTime<Utc>, limit: i64) -> AppResult<Vec<PendingFirebaseDeletion>>;

    /// Remove the row for `firebase_uid`: Firebase confirmed the identity is
    /// gone. Returns whether a row was removed.
    async fn complete(&self, firebase_uid: &str) -> AppResult<bool>;

    /// Record a failed attempt: bump the count, keep `error`, and hold the
    /// row until `next_attempt_at`. Returns the attempt count now recorded,
    /// or `None` when no row exists for the uid.
    async fn record_failure(
        &self,
        firebase_uid: &str,
        error: &str,
        next_attempt_at: DateTime<Utc>,
    ) -> AppResult<Option<i64>>;
}

/// Queue the Firebase identity of the user being deleted, inside the account
/// delete's transaction. `$1` is the user id (bound through the backend's
/// uuid codec), `$2` the Firebase project, `$3` now. Inserts nothing for an
/// account without a Firebase uid; a uid already queued stays as it is.
pub(crate) const QUEUE_FIREBASE_DELETION_SQL: &str = r"
            INSERT INTO firebase_identity_deletions
                (firebase_uid, firebase_project, attempts, next_attempt_at, last_error, created_at)
            SELECT firebase_uid, $2, 0, $3, NULL, $3
            FROM users
            WHERE id = $1 AND firebase_uid IS NOT NULL
            ON CONFLICT (firebase_uid) DO NOTHING
            ";

/// The rows due at `$1`, oldest first, capped at `$2`.
///
/// A uid a live account holds again is not due: the athlete signed in with
/// the same Google account before its identity was deleted, so that identity
/// now backs the new account. Its row waits, and becomes due again once that
/// account is deleted in turn.
pub(crate) const DUE_FIREBASE_DELETIONS_SQL: &str = r"
            SELECT firebase_uid, firebase_project, attempts, next_attempt_at, last_error
            FROM firebase_identity_deletions
            WHERE next_attempt_at <= $1
              AND NOT EXISTS (
                  SELECT 1 FROM users
                  WHERE users.firebase_uid = firebase_identity_deletions.firebase_uid
              )
            ORDER BY next_attempt_at
            LIMIT $2
            ";

/// Remove the row for a uid Firebase no longer holds.
pub(crate) const COMPLETE_FIREBASE_DELETION_SQL: &str =
    "DELETE FROM firebase_identity_deletions WHERE firebase_uid = $1";

/// Count a failed attempt and hold the row until the next one.
pub(crate) const RECORD_FIREBASE_DELETION_FAILURE_SQL: &str = r"
            UPDATE firebase_identity_deletions
            SET attempts = attempts + 1, last_error = $2, next_attempt_at = $3
            WHERE firebase_uid = $1
            RETURNING attempts
            ";

/// The error for a column of this table that would not decode.
pub(crate) fn firebase_deletion_column_error(name: &str, e: impl Display) -> AppError {
    AppError::database(format!(
        "Failed to read firebase_identity_deletions.{name}: {e}"
    ))
}

/// Emit the whole [`FirebaseIdentityDeletionRepository`] implementation for
/// one backend type; sqlx resolves the driver from `self.pool()`.
macro_rules! impl_firebase_identity_deletion_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl FirebaseIdentityDeletionRepository for $ty {
            async fn due(
                &self,
                now: DateTime<Utc>,
                limit: i64,
            ) -> AppResult<Vec<PendingFirebaseDeletion>> {
                let rows = sqlx::query(DUE_FIREBASE_DELETIONS_SQL)
                    .bind(now)
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to list pending Firebase identity deletions: {e}"
                        ))
                    })?;
                rows.iter()
                    .map(|row| {
                        Ok(PendingFirebaseDeletion {
                            firebase_uid: row
                                .try_get("firebase_uid")
                                .map_err(|e| firebase_deletion_column_error("firebase_uid", e))?,
                            firebase_project: row.try_get("firebase_project").map_err(|e| {
                                firebase_deletion_column_error("firebase_project", e)
                            })?,
                            attempts: row
                                .try_get("attempts")
                                .map_err(|e| firebase_deletion_column_error("attempts", e))?,
                            next_attempt_at: row.try_get("next_attempt_at").map_err(|e| {
                                firebase_deletion_column_error("next_attempt_at", e)
                            })?,
                            last_error: row
                                .try_get("last_error")
                                .map_err(|e| firebase_deletion_column_error("last_error", e))?,
                        })
                    })
                    .collect()
            }

            async fn complete(&self, firebase_uid: &str) -> AppResult<bool> {
                let done = sqlx::query(COMPLETE_FIREBASE_DELETION_SQL)
                    .bind(firebase_uid)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to remove a Firebase identity deletion: {e}"
                        ))
                    })?;
                Ok(done.rows_affected() > 0)
            }

            async fn record_failure(
                &self,
                firebase_uid: &str,
                error: &str,
                next_attempt_at: DateTime<Utc>,
            ) -> AppResult<Option<i64>> {
                let row = sqlx::query(RECORD_FIREBASE_DELETION_FAILURE_SQL)
                    .bind(firebase_uid)
                    .bind(error)
                    .bind(next_attempt_at)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to record a Firebase identity deletion failure: {e}"
                        ))
                    })?;
                row.map(|row| {
                    row.try_get("attempts")
                        .map_err(|e| firebase_deletion_column_error("attempts", e))
                })
                .transpose()
            }
        }
    };
}
pub(crate) use impl_firebase_identity_deletion_repository;
