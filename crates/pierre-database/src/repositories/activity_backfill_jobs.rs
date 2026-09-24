// ABOUTME: Repository trait for historical activity backfills still owed — one per (user, provider), recorded before the spawn
// ABOUTME: The SQL is written once here and both backends emit their impl from it; a lapsed lease is what the resume sweep claims

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

/// One backfill owed: the window that was asked for and where its finished
/// answer is expected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityBackfillJobRow {
    /// Job id (UUID string), minted by the recorder.
    pub id: String,
    /// Tenant that owns the cache rows.
    pub tenant_id: TenantId,
    /// Athlete whose history is being backfilled.
    pub user_id: Uuid,
    /// Backend provider slug (already resolved to the sciotte mirror if any).
    pub provider: String,
    /// The requested window's deep `after` (unix seconds).
    pub after_ts: Option<i64>,
    /// The requested window's `before` (unix seconds).
    pub before_ts: Option<i64>,
    /// The requested fetch limit.
    pub fetch_limit: Option<i64>,
    /// Pierre conversation the completion notice goes back to, if any.
    pub conversation_id: Option<String>,
    /// Unix milliseconds when the ask recorded it.
    pub created_at_ms: i64,
    /// Unix milliseconds until which one runner holds it; `0` when free.
    pub leased_until_ms: i64,
    /// Runs started so far, including the one this claim represents.
    pub attempts: i64,
}

/// Which rows a resume sweep may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackfillJobClaim {
    /// The moment the claim is made.
    pub now_ms: i64,
    /// Only rows recorded before `now_ms - queued_older_than_ms` are stale
    /// enough to take: a fresh row is one its own spawn is still running.
    pub queued_older_than_ms: i64,
    /// How long the claim holds the row.
    pub lease_ms: i64,
    /// Rows at or past this many attempts are left alone.
    pub max_attempts: i64,
    /// At most this many rows per sweep.
    pub limit: i64,
}

/// The ledger of backfills owed.
#[async_trait]
pub trait ActivityBackfillJobRepository: Send + Sync {
    /// Record a backfill before its spawn, leased to the recording instance
    /// until `leased_until_ms`. Idempotent on `(user_id, provider)`: returns
    /// `true` when this call inserted the row, `false` when one was already
    /// owed — the same job, already on file.
    async fn record_backfill_job(&self, row: &ActivityBackfillJobRow) -> AppResult<bool>;

    /// Atomically lease up to `claim.limit` rows whose lease has lapsed, that
    /// were recorded before `now_ms - queued_older_than_ms`, and whose
    /// attempts are below the cap — oldest first, incrementing `attempts`.
    /// The returned rows carry the incremented count.
    async fn claim_stale_backfill_jobs(
        &self,
        claim: BackfillJobClaim,
    ) -> AppResult<Vec<ActivityBackfillJobRow>>;

    /// Extend the lease a running backfill holds. Returns `false` when the
    /// row is gone or leased past this runner's claim by someone else.
    async fn renew_backfill_job_lease(&self, id: &str, leased_until_ms: i64) -> AppResult<bool>;

    /// The backfill reached a terminal outcome the athlete has been told
    /// about (completed, or reconnect needed): delete the row.
    async fn finish_backfill_job(&self, id: &str) -> AppResult<()>;

    /// Delete every row at or past the attempt cap whose lease has lapsed:
    /// the run that was to finish it died too, no claim will take it again,
    /// and while it stands `record_backfill_job` refuses every new ask for
    /// the pair. Returns how many were removed.
    async fn reap_exhausted_backfill_jobs(&self, now_ms: i64, max_attempts: i64) -> AppResult<u64>;
}

/// `ON CONFLICT DO NOTHING` on `(user_id, provider)`: a second ask while one
/// is owed is the same job, and `rows_affected` tells the caller so.
pub(crate) const RECORD_BACKFILL_JOB_SQL: &str = "INSERT INTO activity_backfill_jobs \
     (id, tenant_id, user_id, provider, after_ts, before_ts, fetch_limit, \
      conversation_id, created_at_ms, leased_until_ms, attempts) \
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
     ON CONFLICT (user_id, provider) DO NOTHING";

/// The claim: one statement, so the lease and the attempt bump land
/// together and the subquery picks the rows under the same lock. The
/// backend's lock clause — `FOR UPDATE SKIP LOCKED` on `PostgreSQL`, nothing
/// on `SQLite` — is spliced in by the backend shell.
macro_rules! claim_backfill_jobs_sql {
    ($lock:literal) => {
        concat!(
            "UPDATE activity_backfill_jobs \
             SET leased_until_ms = $1, attempts = attempts + 1 \
             WHERE id IN ( \
                 SELECT j.id FROM activity_backfill_jobs j \
                 WHERE j.leased_until_ms <= $2 \
                   AND j.created_at_ms < $3 \
                   AND j.attempts < $4 \
                 ORDER BY j.created_at_ms ASC \
                 LIMIT $5 ",
            $lock,
            ") \
             RETURNING id, tenant_id, user_id, provider, after_ts, before_ts, fetch_limit, \
                       conversation_id, created_at_ms, leased_until_ms, attempts"
        )
    };
}
pub(crate) use claim_backfill_jobs_sql;

/// A renewal only ever moves the lease later: one that would shorten it is
/// not this runner's row any more.
pub(crate) const RENEW_BACKFILL_JOB_LEASE_SQL: &str =
    "UPDATE activity_backfill_jobs SET leased_until_ms = $1 \
     WHERE id = $2 AND leased_until_ms <= $1";

pub(crate) const FINISH_BACKFILL_JOB_SQL: &str = "DELETE FROM activity_backfill_jobs WHERE id = $1";

pub(crate) const REAP_EXHAUSTED_BACKFILL_JOBS_SQL: &str =
    "DELETE FROM activity_backfill_jobs WHERE attempts >= $1 AND leased_until_ms <= $2";

pub(crate) fn backfill_job_from_row<R>(row: &R) -> AppResult<ActivityBackfillJobRow>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<i64>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    TenantId: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column = |name: &str, e: sqlx::Error| {
        AppError::database(format!("activity_backfill_jobs {name}: {e}"))
    };
    let user_id: String = row.try_get("user_id").map_err(|e| column("user_id", e))?;
    Ok(ActivityBackfillJobRow {
        id: row.try_get("id").map_err(|e| column("id", e))?,
        tenant_id: row
            .try_get("tenant_id")
            .map_err(|e| column("tenant_id", e))?,
        user_id: Uuid::parse_str(&user_id)
            .map_err(|e| AppError::database(format!("backfill job user_id is not a UUID: {e}")))?,
        provider: row.try_get("provider").map_err(|e| column("provider", e))?,
        after_ts: row.try_get("after_ts").map_err(|e| column("after_ts", e))?,
        before_ts: row
            .try_get("before_ts")
            .map_err(|e| column("before_ts", e))?,
        fetch_limit: row
            .try_get("fetch_limit")
            .map_err(|e| column("fetch_limit", e))?,
        conversation_id: row
            .try_get("conversation_id")
            .map_err(|e| column("conversation_id", e))?,
        created_at_ms: row
            .try_get("created_at_ms")
            .map_err(|e| column("created_at_ms", e))?,
        leased_until_ms: row
            .try_get("leased_until_ms")
            .map_err(|e| column("leased_until_ms", e))?,
        attempts: row.try_get("attempts").map_err(|e| column("attempts", e))?,
    })
}

/// Emit the whole [`ActivityBackfillJobRepository`] implementation for one
/// backend type. The body is written once here; each backend's shell invokes
/// it with its own type and its lock clause for [`claim_backfill_jobs_sql!`],
/// and sqlx resolves the driver from `self.pool()`.
///
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
macro_rules! impl_activity_backfill_job_repository {
    ($ty:ty, $lock:literal) => {
        #[async_trait::async_trait]
        impl ActivityBackfillJobRepository for $ty {
            async fn record_backfill_job(&self, row: &ActivityBackfillJobRow) -> AppResult<bool> {
                let result = sqlx::query(RECORD_BACKFILL_JOB_SQL)
                    .bind(&row.id)
                    .bind(row.tenant_id)
                    .bind(row.user_id.to_string())
                    .bind(&row.provider)
                    .bind(row.after_ts)
                    .bind(row.before_ts)
                    .bind(row.fetch_limit)
                    .bind(&row.conversation_id)
                    .bind(row.created_at_ms)
                    .bind(row.leased_until_ms)
                    .bind(row.attempts)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record backfill job: {e}"))
                    })?;
                Ok(result.rows_affected() == 1)
            }

            async fn claim_stale_backfill_jobs(
                &self,
                claim: BackfillJobClaim,
            ) -> AppResult<Vec<ActivityBackfillJobRow>> {
                let rows = sqlx::query(claim_backfill_jobs_sql!($lock))
                    .bind(claim.now_ms.saturating_add(claim.lease_ms))
                    .bind(claim.now_ms)
                    .bind(claim.now_ms.saturating_sub(claim.queued_older_than_ms))
                    .bind(claim.max_attempts)
                    .bind(claim.limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to claim backfill jobs: {e}"))
                    })?;
                rows.iter().map(backfill_job_from_row).collect()
            }

            async fn renew_backfill_job_lease(
                &self,
                id: &str,
                leased_until_ms: i64,
            ) -> AppResult<bool> {
                let result = sqlx::query(RENEW_BACKFILL_JOB_LEASE_SQL)
                    .bind(leased_until_ms)
                    .bind(id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to renew backfill job lease: {e}"))
                    })?;
                Ok(result.rows_affected() == 1)
            }

            async fn finish_backfill_job(&self, id: &str) -> AppResult<()> {
                sqlx::query(FINISH_BACKFILL_JOB_SQL)
                    .bind(id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to finish backfill job: {e}"))
                    })?;
                Ok(())
            }

            async fn reap_exhausted_backfill_jobs(
                &self,
                now_ms: i64,
                max_attempts: i64,
            ) -> AppResult<u64> {
                let outcome = sqlx::query(REAP_EXHAUSTED_BACKFILL_JOBS_SQL)
                    .bind(max_attempts)
                    .bind(now_ms)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to reap exhausted backfill jobs: {e}"))
                    })?;
                Ok(outcome.rows_affected())
            }
        }
    };
}
pub(crate) use impl_activity_backfill_job_repository;
