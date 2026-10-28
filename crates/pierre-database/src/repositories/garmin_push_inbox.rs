// ABOUTME: Repository trait for the Garmin push inbox — every pushed batch, written before the 200, worked under a lease
// ABOUTME: The SQL is written once here and both backends emit their impl from it; a lapsed lease is what the sweep claims

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The Garmin push inbox.
//!
//! Garmin wants a 200 within thirty seconds of each push, before any
//! processing, and does not resend a batch it saw answered. The route writes
//! the raw batch here before it answers ([`GarminPushInboxRepository::record_push`]),
//! leased to the instance that received it, which then processes it at once.
//! When that instance dies first the lease lapses and the sweep claims the row
//! ([`GarminPushInboxRepository::claim_due_pushes`]). A run that fails puts the
//! row back with a backoff; one that keeps failing ends `failed`.
//!
//! A batch names several Garmin users and carries no tenant, so the table has
//! no user or tenant column: each record's owner is resolved when the row is
//! processed. Every timestamp is unix milliseconds in an integer column, the
//! same on both engines.

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use std::fmt;

use crate::repositories::memory_extraction_jobs::JobLease;

/// The `status` of a row still to process.
pub const INBOX_PENDING: &str = "pending";
/// The `status` of a row whose records all landed.
pub const INBOX_DONE: &str = "done";
/// The `status` of a row given up on after its last attempt.
pub const INBOX_FAILED: &str = "failed";

/// One pushed batch.
#[derive(Clone, PartialEq, Eq)]
pub struct GarminPushInboxRow {
    /// Row id (UUID string).
    pub id: String,
    /// The summary type the batch carries (`activities`, `sleeps`, …): the
    /// key of the body's one list.
    pub summary_type: String,
    /// When the push was received, unix milliseconds.
    pub received_at_ms: i64,
    /// The raw JSON body Garmin posted; empty once the row is processed.
    pub payload: String,
    /// [`INBOX_PENDING`], [`INBOX_DONE`] or [`INBOX_FAILED`].
    pub status: String,
    /// Runs started so far, including the one this claim represents.
    pub attempts: i64,
    /// Why the last run failed, without any record's content.
    pub last_error: Option<String>,
    /// When the row reached `done` or `failed`, unix milliseconds.
    pub processed_at_ms: Option<i64>,
    /// Unix milliseconds until which one runner holds the row; `0` when free.
    pub leased_until_ms: i64,
    /// The token of the claim that holds the row.
    pub lease_token: Option<String>,
}

/// Prints the bookkeeping only: the payload is an athlete's health data and
/// the lease token grants the right to finish the row.
impl fmt::Debug for GarminPushInboxRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GarminPushInboxRow")
            .field("id", &self.id)
            .field("summary_type", &self.summary_type)
            .field("received_at_ms", &self.received_at_ms)
            .field("payload_bytes", &self.payload.len())
            .field("status", &self.status)
            .field("attempts", &self.attempts)
            .field("last_error", &self.last_error)
            .field("processed_at_ms", &self.processed_at_ms)
            .field("leased_until_ms", &self.leased_until_ms)
            .field("lease_token", &"[redacted]")
            .finish()
    }
}

/// Which rows a sweep may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InboxClaim {
    /// The moment the claim is made.
    pub now_ms: i64,
    /// How long the claim holds each row.
    pub lease_ms: i64,
    /// Rows at or past this many attempts are left alone.
    pub max_attempts: i64,
    /// At most this many rows per claim.
    pub limit: i64,
}

/// The inbox of pushed Garmin batches.
#[async_trait]
pub trait GarminPushInboxRepository: Send + Sync {
    /// Record a received batch as pending, already leased to the receiving
    /// instance (`row.leased_until_ms`, `row.lease_token`, `row.attempts`),
    /// so the sweep does not race the run the route starts.
    async fn record_push(&self, row: &GarminPushInboxRow) -> AppResult<()>;

    /// Atomically lease up to `claim.limit` pending rows whose lease lapsed
    /// and whose attempts are below the cap, oldest first, incrementing
    /// `attempts`. The returned rows carry the new token and count.
    async fn claim_due_pushes(&self, claim: InboxClaim) -> AppResult<Vec<GarminPushInboxRow>>;

    /// The row is settled: mark it `done` at `now_ms` and blank its payload,
    /// if this lease still holds it. `note` records what it settled without
    /// (records that waited on a push that never came), `None` when every
    /// record landed. `false` when another claim took it.
    async fn finish_push(
        &self,
        lease: &JobLease,
        now_ms: i64,
        note: Option<&str>,
    ) -> AppResult<bool>;

    /// The run failed with `error`: hand the row back for another run at
    /// `retry_at_ms`, if this lease still holds it. `false` when another
    /// claim took it.
    async fn retry_push(&self, lease: &JobLease, error: &str, retry_at_ms: i64) -> AppResult<bool>;

    /// The run failed for the last time: mark the row `failed` at `now_ms`
    /// with `error` and blank its payload, if this lease still holds it.
    async fn fail_push(&self, lease: &JobLease, error: &str, now_ms: i64) -> AppResult<bool>;

    /// Mark `failed` every pending row at or past the attempt cap whose
    /// lease lapsed: its last runner died, and no claim will take it again.
    /// Returns how many were given up on.
    async fn give_up_exhausted_pushes(&self, now_ms: i64, max_attempts: i64) -> AppResult<u64>;

    /// Delete every `done` or `failed` row processed before `cutoff_ms`.
    /// Returns how many went.
    async fn delete_processed_pushes(&self, cutoff_ms: i64) -> AppResult<u64>;

    /// One row by id, for the operator and the tests.
    async fn get_push(&self, id: &str) -> AppResult<Option<GarminPushInboxRow>>;
}

pub(crate) const RECORD_PUSH_SQL: &str = "INSERT INTO garmin_push_inbox \
     (id, summary_type, received_at_ms, payload, status, attempts, leased_until_ms, lease_token) \
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8)";

/// The claim: one statement, so the lease and the attempt bump land together
/// and the subquery picks the rows under the same lock. The backend's lock
/// clause — `FOR UPDATE SKIP LOCKED` on `PostgreSQL`, nothing on `SQLite` —
/// is spliced in by the backend shell.
macro_rules! claim_garmin_pushes_sql {
    ($lock:literal) => {
        concat!(
            "UPDATE garmin_push_inbox \
             SET leased_until_ms = $1, attempts = attempts + 1, lease_token = $5 \
             WHERE id IN ( \
                 SELECT i.id FROM garmin_push_inbox i \
                 WHERE i.status = 'pending' \
                   AND i.leased_until_ms <= $2 \
                   AND i.attempts < $3 \
                 ORDER BY i.received_at_ms ASC \
                 LIMIT $4 ",
            $lock,
            ") \
             RETURNING id, summary_type, received_at_ms, payload, status, attempts, last_error, \
                       processed_at_ms, leased_until_ms, lease_token"
        )
    };
}
pub(crate) use claim_garmin_pushes_sql;

pub(crate) const FINISH_PUSH_SQL: &str = "UPDATE garmin_push_inbox \
     SET status = 'done', processed_at_ms = $1, payload = '', last_error = $4, \
         leased_until_ms = 0, lease_token = NULL \
     WHERE id = $2 AND lease_token = $3";

pub(crate) const RETRY_PUSH_SQL: &str = "UPDATE garmin_push_inbox \
     SET last_error = $1, leased_until_ms = $2, lease_token = NULL \
     WHERE id = $3 AND lease_token = $4";

pub(crate) const FAIL_PUSH_SQL: &str = "UPDATE garmin_push_inbox \
     SET status = 'failed', processed_at_ms = $1, payload = '', last_error = $2, \
         leased_until_ms = 0, lease_token = NULL \
     WHERE id = $3 AND lease_token = $4";

pub(crate) const GIVE_UP_EXHAUSTED_PUSHES_SQL: &str = "UPDATE garmin_push_inbox \
     SET status = 'failed', processed_at_ms = $1, payload = '', lease_token = NULL \
     WHERE status = 'pending' AND attempts >= $2 AND leased_until_ms <= $1";

pub(crate) const DELETE_PROCESSED_PUSHES_SQL: &str = "DELETE FROM garmin_push_inbox \
     WHERE status IN ('done', 'failed') AND processed_at_ms < $1";

pub(crate) const GET_PUSH_SQL: &str = "SELECT id, summary_type, received_at_ms, payload, status, \
     attempts, last_error, processed_at_ms, leased_until_ms, lease_token \
     FROM garmin_push_inbox WHERE id = $1";

pub(crate) fn push_from_row<R>(row: &R) -> AppResult<GarminPushInboxRow>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<i64>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column =
        |name: &str, e: sqlx::Error| AppError::database(format!("garmin_push_inbox {name}: {e}"));
    Ok(GarminPushInboxRow {
        id: row.try_get("id").map_err(|e| column("id", e))?,
        summary_type: row
            .try_get("summary_type")
            .map_err(|e| column("summary_type", e))?,
        received_at_ms: row
            .try_get("received_at_ms")
            .map_err(|e| column("received_at_ms", e))?,
        payload: row.try_get("payload").map_err(|e| column("payload", e))?,
        status: row.try_get("status").map_err(|e| column("status", e))?,
        attempts: row.try_get("attempts").map_err(|e| column("attempts", e))?,
        last_error: row
            .try_get("last_error")
            .map_err(|e| column("last_error", e))?,
        processed_at_ms: row
            .try_get("processed_at_ms")
            .map_err(|e| column("processed_at_ms", e))?,
        leased_until_ms: row
            .try_get("leased_until_ms")
            .map_err(|e| column("leased_until_ms", e))?,
        lease_token: row
            .try_get("lease_token")
            .map_err(|e| column("lease_token", e))?,
    })
}

/// Emit the whole [`GarminPushInboxRepository`] implementation for one
/// backend type. The body is written once here; each backend's shell invokes
/// it with its own type and its lock clause for [`claim_garmin_pushes_sql!`],
/// and sqlx resolves the driver from `self.pool()`.
///
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
macro_rules! impl_garmin_push_inbox_repository {
    ($ty:ty, $lock:literal) => {
        #[async_trait::async_trait]
        impl GarminPushInboxRepository for $ty {
            async fn record_push(&self, row: &GarminPushInboxRow) -> AppResult<()> {
                sqlx::query(RECORD_PUSH_SQL)
                    .bind(&row.id)
                    .bind(&row.summary_type)
                    .bind(row.received_at_ms)
                    .bind(&row.payload)
                    .bind(&row.status)
                    .bind(row.attempts)
                    .bind(row.leased_until_ms)
                    .bind(&row.lease_token)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record Garmin push: {e}"))
                    })?;
                Ok(())
            }

            async fn claim_due_pushes(
                &self,
                claim: InboxClaim,
            ) -> AppResult<Vec<GarminPushInboxRow>> {
                let rows = sqlx::query(claim_garmin_pushes_sql!($lock))
                    .bind(claim.now_ms.saturating_add(claim.lease_ms))
                    .bind(claim.now_ms)
                    .bind(claim.max_attempts)
                    .bind(claim.limit)
                    .bind(uuid::Uuid::new_v4().to_string())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to claim Garmin pushes: {e}"))
                    })?;
                rows.iter().map(push_from_row).collect()
            }

            async fn finish_push(
                &self,
                lease: &JobLease,
                now_ms: i64,
                note: Option<&str>,
            ) -> AppResult<bool> {
                let done = sqlx::query(FINISH_PUSH_SQL)
                    .bind(now_ms)
                    .bind(&lease.id)
                    .bind(&lease.token)
                    .bind(note)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to finish Garmin push: {e}"))
                    })?;
                Ok(done.rows_affected() == 1)
            }

            async fn retry_push(
                &self,
                lease: &JobLease,
                error: &str,
                retry_at_ms: i64,
            ) -> AppResult<bool> {
                let done = sqlx::query(RETRY_PUSH_SQL)
                    .bind(error)
                    .bind(retry_at_ms)
                    .bind(&lease.id)
                    .bind(&lease.token)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to retry Garmin push: {e}")))?;
                Ok(done.rows_affected() == 1)
            }

            async fn fail_push(
                &self,
                lease: &JobLease,
                error: &str,
                now_ms: i64,
            ) -> AppResult<bool> {
                let done = sqlx::query(FAIL_PUSH_SQL)
                    .bind(now_ms)
                    .bind(error)
                    .bind(&lease.id)
                    .bind(&lease.token)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to fail Garmin push: {e}")))?;
                Ok(done.rows_affected() == 1)
            }

            async fn give_up_exhausted_pushes(
                &self,
                now_ms: i64,
                max_attempts: i64,
            ) -> AppResult<u64> {
                let outcome = sqlx::query(GIVE_UP_EXHAUSTED_PUSHES_SQL)
                    .bind(now_ms)
                    .bind(max_attempts)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to give up exhausted Garmin pushes: {e}"
                        ))
                    })?;
                Ok(outcome.rows_affected())
            }

            async fn delete_processed_pushes(&self, cutoff_ms: i64) -> AppResult<u64> {
                let outcome = sqlx::query(DELETE_PROCESSED_PUSHES_SQL)
                    .bind(cutoff_ms)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete processed Garmin pushes: {e}"))
                    })?;
                Ok(outcome.rows_affected())
            }

            async fn get_push(&self, id: &str) -> AppResult<Option<GarminPushInboxRow>> {
                let row = sqlx::query(GET_PUSH_SQL)
                    .bind(id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to read Garmin push: {e}")))?;
                row.as_ref().map(push_from_row).transpose()
            }
        }
    };
}
pub(crate) use impl_garmin_push_inbox_repository;
