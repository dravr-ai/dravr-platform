// ABOUTME: Repository trait for historical activity backfills still owed — one per (user, provider), recorded before the spawn
// ABOUTME: The SQL is written once here and both backends emit their impl from it; a lapsed lease is what the resume sweep claims

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use std::fmt;
use uuid::Uuid;

use crate::repositories::memory_extraction_jobs::JobLease;

/// One backfill owed: the window that was asked for and where its finished
/// answer is expected.
#[derive(Clone, PartialEq, Eq)]
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
    /// The token of the claim (or the recording) that holds the row; renewing
    /// and finishing it require this token. `None` on a row recorded before
    /// tokens existed, which only a claim can take.
    pub lease_token: Option<String>,
    /// Whether the job is a usr/pwd session's one-time history import
    /// (carnet#1386), as the ask that started it — or adopted it — recorded.
    pub history_import: bool,
}

/// Prints every field but the lease token, which grants the right to renew
/// and finish the row and so stays out of logs and panics.
impl fmt::Debug for ActivityBackfillJobRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActivityBackfillJobRow")
            .field("id", &self.id)
            .field("tenant_id", &self.tenant_id)
            .field("user_id", &self.user_id)
            .field("provider", &self.provider)
            .field("after_ts", &self.after_ts)
            .field("before_ts", &self.before_ts)
            .field("fetch_limit", &self.fetch_limit)
            .field("conversation_id", &self.conversation_id)
            .field("created_at_ms", &self.created_at_ms)
            .field("leased_until_ms", &self.leased_until_ms)
            .field("attempts", &self.attempts)
            .field("lease_token", &"[redacted]")
            .field("history_import", &self.history_import)
            .finish()
    }
}

/// What the ledger holds for one `(user, provider)` pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfillJobState {
    /// A job is owed: recorded, running or waiting for the resume sweep.
    Owed,
    /// The job ended before what it was asked for landed, and its row was
    /// kept to say so ([`ActivityBackfillJobRepository::end_backfill_job_incomplete`]):
    /// a one-time history import whose session expired mid-copy.
    Incomplete,
}

/// The `outcome` an ended row carries in [`BackfillJobState::Incomplete`].
const OUTCOME_INCOMPLETE: &str = "incomplete";

/// The `kind` a one-time history import's row carries; `NULL` is an
/// ordinary backfill.
const KIND_HISTORY_IMPORT: &str = "history_import";

/// The window a history import asks for when it adopts a job already owed
/// for its session ([`ActivityBackfillJobRepository::adopt_as_history_import`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryImportWindow {
    /// The import's deep `after` (unix seconds).
    pub after_ts: i64,
    /// The import's fetch limit.
    pub fetch_limit: Option<i64>,
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
    /// owed — the same job, already on file. A row that only records how an
    /// earlier job ended ([`BackfillJobState::Incomplete`]) is replaced by an
    /// ask in its own tenant: it owes nothing. An ask from another tenant
    /// leaves it, and returns `false`.
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
    /// row is gone or another claim holds it.
    async fn renew_backfill_job_lease(
        &self,
        lease: &JobLease,
        leased_until_ms: i64,
    ) -> AppResult<bool>;

    /// The backfill reached a terminal outcome the athlete has been told
    /// about (completed, or reconnect needed): delete the row, if this lease
    /// still holds it. `false` when another claim took it.
    async fn finish_backfill_job(&self, lease: &JobLease) -> AppResult<bool>;

    /// The job ended before what it was asked for landed: keep its row, if
    /// this lease still holds it, as [`BackfillJobState::Incomplete`] —
    /// released, never claimed or reaped again. `false` when another claim
    /// took it.
    async fn end_backfill_job_incomplete(&self, lease: &JobLease) -> AppResult<bool>;

    /// Give back a claim the sweep could not run (a run for the pair is
    /// already in flight on this instance): the row is released and the
    /// attempt the claim counted is returned, so waiting on that run never
    /// spends the job's attempts. `false` when this lease no longer holds it.
    async fn release_backfill_job_claim(&self, lease: &JobLease) -> AppResult<bool>;

    /// A history import filled its fetch limit: keep its row owed for the
    /// next page, `before_ts` the oldest activity this page reached, with its
    /// attempts reset and its lease released so the resume sweep runs that
    /// page. `false` when this lease no longer holds the row.
    async fn page_backfill_job_down(&self, lease: &JobLease, before_ts: i64) -> AppResult<bool>;

    /// Turn the ordinary job owed for `(user_id, provider)` in `tenant_id`
    /// into a history import over `window`: no `before`, no conversation,
    /// attempts reset, and its lease token dropped, so the run that holds it
    /// now can neither renew nor finish it and the resume sweep runs it as
    /// the import once that lease lapses. `false` when no ordinary job is
    /// owed for the pair there.
    async fn adopt_as_history_import(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        window: HistoryImportWindow,
    ) -> AppResult<bool>;

    /// What the ledger holds for `(user_id, provider)` in `tenant_id`, or
    /// `None` when it holds nothing.
    async fn backfill_job_state(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<Option<BackfillJobState>>;

    /// Delete every row at or past the attempt cap whose lease has lapsed:
    /// the run that was to finish it died too, no claim will take it again,
    /// and while it stands `record_backfill_job` refuses every new ask for
    /// the pair. Returns how many were removed.
    async fn reap_exhausted_backfill_jobs(&self, now_ms: i64, max_attempts: i64) -> AppResult<u64>;
}

/// A conflict on `(user_id, provider)` changes nothing while a job is owed
/// (`outcome IS NULL`): a second ask is the same job, and `rows_affected`
/// tells the caller so. A row that only records an ended job is replaced
/// whole by the new one, but only by an ask in the row's own tenant: an ask
/// from another tenant never overwrites what this tenant's card reads.
pub(crate) const RECORD_BACKFILL_JOB_SQL: &str = "INSERT INTO activity_backfill_jobs \
     (id, tenant_id, user_id, provider, after_ts, before_ts, fetch_limit, \
      conversation_id, created_at_ms, leased_until_ms, attempts, lease_token, kind) \
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
     ON CONFLICT (user_id, provider) DO UPDATE SET \
       id = excluded.id, tenant_id = excluded.tenant_id, after_ts = excluded.after_ts, \
       before_ts = excluded.before_ts, fetch_limit = excluded.fetch_limit, \
       conversation_id = excluded.conversation_id, created_at_ms = excluded.created_at_ms, \
       leased_until_ms = excluded.leased_until_ms, attempts = excluded.attempts, \
       lease_token = excluded.lease_token, kind = excluded.kind, outcome = NULL \
     WHERE activity_backfill_jobs.outcome IS NOT NULL \
       AND activity_backfill_jobs.tenant_id = excluded.tenant_id";

/// The claim: one statement, so the lease and the attempt bump land
/// together and the subquery picks the rows under the same lock. The
/// backend's lock clause — `FOR UPDATE SKIP LOCKED` on `PostgreSQL`, nothing
/// on `SQLite` — is spliced in by the backend shell.
macro_rules! claim_backfill_jobs_sql {
    ($lock:literal) => {
        concat!(
            "UPDATE activity_backfill_jobs \
             SET leased_until_ms = $1, attempts = attempts + 1, lease_token = $6 \
             WHERE id IN ( \
                 SELECT j.id FROM activity_backfill_jobs j \
                 WHERE j.leased_until_ms <= $2 \
                   AND j.created_at_ms < $3 \
                   AND j.attempts < $4 \
                   AND j.outcome IS NULL \
                 ORDER BY j.created_at_ms ASC \
                 LIMIT $5 ",
            $lock,
            ") \
             RETURNING id, tenant_id, user_id, provider, after_ts, before_ts, fetch_limit, \
                       conversation_id, created_at_ms, leased_until_ms, attempts, lease_token, \
                       kind"
        )
    };
}
pub(crate) use claim_backfill_jobs_sql;

/// A renewal applies only while this runner's claim holds the row.
pub(crate) const RENEW_BACKFILL_JOB_LEASE_SQL: &str =
    "UPDATE activity_backfill_jobs SET leased_until_ms = $1 \
     WHERE id = $2 AND lease_token = $3";

pub(crate) const FINISH_BACKFILL_JOB_SQL: &str =
    "DELETE FROM activity_backfill_jobs WHERE id = $1 AND lease_token = $2";

pub(crate) const REAP_EXHAUSTED_BACKFILL_JOBS_SQL: &str = "DELETE FROM activity_backfill_jobs \
     WHERE attempts >= $1 AND leased_until_ms <= $2 AND outcome IS NULL";

/// Keeps the row, released, with the outcome it ended on.
pub(crate) const END_BACKFILL_JOB_SQL: &str = "UPDATE activity_backfill_jobs \
     SET outcome = $3, leased_until_ms = 0, lease_token = NULL \
     WHERE id = $1 AND lease_token = $2";

/// Hands a claim back unrun, with the attempt it counted.
pub(crate) const RELEASE_BACKFILL_JOB_CLAIM_SQL: &str = "UPDATE activity_backfill_jobs \
     SET leased_until_ms = 0, attempts = CASE WHEN attempts > 0 THEN attempts - 1 ELSE 0 END, \
         lease_token = NULL \
     WHERE id = $1 AND lease_token = $2 AND outcome IS NULL";

/// Releases the row for its next page, below the oldest activity reached.
pub(crate) const PAGE_BACKFILL_JOB_DOWN_SQL: &str = "UPDATE activity_backfill_jobs \
     SET before_ts = $3, attempts = 0, leased_until_ms = 0, lease_token = NULL \
     WHERE id = $1 AND lease_token = $2 AND outcome IS NULL";

/// Only an ordinary job still owed is adopted; the lease's expiry stays, so
/// the run holding it keeps the pair until then.
pub(crate) const ADOPT_AS_HISTORY_IMPORT_SQL: &str = "UPDATE activity_backfill_jobs \
     SET kind = $4, after_ts = $5, before_ts = NULL, fetch_limit = $6, \
         conversation_id = NULL, attempts = 0, lease_token = NULL \
     WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 \
       AND outcome IS NULL AND kind IS NULL";

pub(crate) const BACKFILL_JOB_STATE_SQL: &str = "SELECT outcome FROM activity_backfill_jobs \
     WHERE user_id = $1 AND tenant_id = $2 AND provider = $3";

/// The state an `outcome` column reads as.
pub(crate) fn backfill_job_state_of(outcome: Option<&str>) -> AppResult<BackfillJobState> {
    match outcome {
        None => Ok(BackfillJobState::Owed),
        Some(OUTCOME_INCOMPLETE) => Ok(BackfillJobState::Incomplete),
        Some(other) => Err(AppError::database(format!(
            "activity_backfill_jobs outcome is not one this build knows: {other}"
        ))),
    }
}

/// The `kind` a history import's row is stored with.
pub(crate) const fn history_import_kind() -> &'static str {
    KIND_HISTORY_IMPORT
}

/// The `kind` column `row` is stored with.
pub(crate) const fn kind_of(row: &ActivityBackfillJobRow) -> Option<&'static str> {
    if row.history_import {
        Some(KIND_HISTORY_IMPORT)
    } else {
        None
    }
}

/// The `outcome` [`BackfillJobState::Incomplete`] is stored as.
pub(crate) const fn incomplete_outcome() -> &'static str {
    OUTCOME_INCOMPLETE
}

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
    let kind: Option<String> = row.try_get("kind").map_err(|e| column("kind", e))?;
    let history_import = match kind.as_deref() {
        None => false,
        Some(KIND_HISTORY_IMPORT) => true,
        Some(other) => {
            return Err(AppError::database(format!(
                "activity_backfill_jobs kind is not one this build knows: {other}"
            )))
        }
    };
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
        lease_token: row
            .try_get("lease_token")
            .map_err(|e| column("lease_token", e))?,
        history_import,
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
                    .bind(&row.lease_token)
                    .bind(kind_of(row))
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
                    .bind(uuid::Uuid::new_v4().to_string())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to claim backfill jobs: {e}"))
                    })?;
                rows.iter().map(backfill_job_from_row).collect()
            }

            async fn renew_backfill_job_lease(
                &self,
                lease: &JobLease,
                leased_until_ms: i64,
            ) -> AppResult<bool> {
                let result = sqlx::query(RENEW_BACKFILL_JOB_LEASE_SQL)
                    .bind(leased_until_ms)
                    .bind(&lease.id)
                    .bind(&lease.token)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to renew backfill job lease: {e}"))
                    })?;
                Ok(result.rows_affected() == 1)
            }

            async fn finish_backfill_job(&self, lease: &JobLease) -> AppResult<bool> {
                let done = sqlx::query(FINISH_BACKFILL_JOB_SQL)
                    .bind(&lease.id)
                    .bind(&lease.token)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to finish backfill job: {e}"))
                    })?;
                Ok(done.rows_affected() == 1)
            }

            async fn end_backfill_job_incomplete(&self, lease: &JobLease) -> AppResult<bool> {
                let ended = sqlx::query(END_BACKFILL_JOB_SQL)
                    .bind(&lease.id)
                    .bind(&lease.token)
                    .bind(incomplete_outcome())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to end backfill job: {e}")))?;
                Ok(ended.rows_affected() == 1)
            }

            async fn release_backfill_job_claim(&self, lease: &JobLease) -> AppResult<bool> {
                let released = sqlx::query(RELEASE_BACKFILL_JOB_CLAIM_SQL)
                    .bind(&lease.id)
                    .bind(&lease.token)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to release backfill job claim: {e}"))
                    })?;
                Ok(released.rows_affected() == 1)
            }

            async fn page_backfill_job_down(
                &self,
                lease: &JobLease,
                before_ts: i64,
            ) -> AppResult<bool> {
                let paged = sqlx::query(PAGE_BACKFILL_JOB_DOWN_SQL)
                    .bind(&lease.id)
                    .bind(&lease.token)
                    .bind(before_ts)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to page backfill job down: {e}"))
                    })?;
                Ok(paged.rows_affected() == 1)
            }

            async fn adopt_as_history_import(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
                window: HistoryImportWindow,
            ) -> AppResult<bool> {
                let adopted = sqlx::query(ADOPT_AS_HISTORY_IMPORT_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id)
                    .bind(provider)
                    .bind(history_import_kind())
                    .bind(window.after_ts)
                    .bind(window.fetch_limit)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to adopt backfill job: {e}"))
                    })?;
                Ok(adopted.rows_affected() == 1)
            }

            async fn backfill_job_state(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                provider: &str,
            ) -> AppResult<Option<BackfillJobState>> {
                let row = sqlx::query(BACKFILL_JOB_STATE_SQL)
                    .bind(user_id.to_string())
                    .bind(tenant_id)
                    .bind(provider)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read backfill job state: {e}"))
                    })?;
                row.map(|row| {
                    let outcome: Option<String> = row.try_get("outcome").map_err(|e| {
                        AppError::database(format!("activity_backfill_jobs outcome: {e}"))
                    })?;
                    backfill_job_state_of(outcome.as_deref())
                })
                .transpose()
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
