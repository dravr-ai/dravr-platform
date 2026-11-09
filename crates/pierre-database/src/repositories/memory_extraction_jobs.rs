// ABOUTME: Repository trait for post-turn memory extractions still owed — recorded before the spawn, deleted on success
// ABOUTME: The SQL is written once here and both backends emit their impl from it; a lapsed lease is what the resume sweep claims

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;

use async_trait::async_trait;
use pierre_core::admin::scope::AdminScope;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use serde::Serialize;

/// One extraction owed: the request the turn produced, as JSON, plus who
/// it is for and how many times it has been attempted.
#[derive(Clone, PartialEq, Eq)]
pub struct MemoryExtractionJobRow {
    /// Job id (UUID string), minted by the recorder.
    pub id: String,
    /// Tenant the facts are stamped under.
    pub tenant_id: TenantId,
    /// Tenant the extraction's `llm_usage` row is billed under: the
    /// conversation's, like every other row of the turn. It differs from
    /// [`Self::tenant_id`] on a guided answer in a shared room, whose facts go
    /// to the athlete's own tenant. `None` on a row recorded before the column
    /// existed, which bills under [`Self::tenant_id`].
    pub usage_tenant_id: Option<TenantId>,
    /// The extraction request, serialised by `pierre-services`.
    pub payload: String,
    /// Unix milliseconds when the turn recorded it.
    pub created_at_ms: i64,
    /// Unix milliseconds until which one runner holds it; `0` when free.
    pub leased_until_ms: i64,
    /// Runs started so far, including the one this claim represents.
    pub attempts: i64,
    /// The token of the claim (or the recording) that holds the row; renewing
    /// and finishing it require this token. `None` on a row recorded before
    /// tokens existed, which only a claim can take.
    pub lease_token: Option<String>,
}

/// Prints every field but the lease token, which grants the right to renew
/// and finish the row and so stays out of logs and panics.
impl fmt::Debug for MemoryExtractionJobRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryExtractionJobRow")
            .field("id", &self.id)
            .field("tenant_id", &self.tenant_id)
            .field("usage_tenant_id", &self.usage_tenant_id)
            .field("payload", &self.payload)
            .field("created_at_ms", &self.created_at_ms)
            .field("leased_until_ms", &self.leased_until_ms)
            .field("attempts", &self.attempts)
            .field("lease_token", &"[redacted]")
            .finish()
    }
}

/// A job row as one runner holds it: its id and the token of its lease.
///
/// Renewing and finishing a job name both, so a runner whose lease lapsed
/// while another claimed the row changes nothing.
#[derive(Clone, PartialEq, Eq)]
pub struct JobLease {
    /// The job row's id.
    pub id: String,
    /// The token the claim or the recording wrote.
    pub token: String,
}

/// Prints every field but the lease token, which grants the right to renew
/// and finish the row and so stays out of logs and panics.
impl fmt::Debug for JobLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JobLease")
            .field("id", &self.id)
            .field("token", &"[redacted]")
            .finish()
    }
}

impl fmt::Display for JobLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.id)
    }
}

/// Which rows a resume sweep may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractionJobClaim {
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

/// How one extraction run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractionRunOutcome {
    /// The run wrote at least one fact.
    Persisted,
    /// The run finished and had nothing to write.
    Empty,
    /// The run could not finish; its job row waits for a retry.
    Failed,
}

impl ExtractionRunOutcome {
    /// The value stored in `memory_extraction_runs.outcome`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Persisted => "persisted",
            Self::Empty => "empty",
            Self::Failed => "failed",
        }
    }
}

/// One extraction run, for the run ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractionRunRecord {
    /// Tenant the run stamps its facts under.
    pub tenant_id: TenantId,
    /// How it ended.
    pub outcome: ExtractionRunOutcome,
    /// Facts it wrote.
    pub facts_persisted: i64,
    /// Unix milliseconds when it ended.
    pub finished_at_ms: i64,
}

/// Runs over one window, by how they ended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ExtractionRunCounts {
    /// Runs that wrote at least one fact.
    pub persisted: i64,
    /// Runs that had nothing to write.
    pub empty: i64,
    /// Runs that could not finish.
    pub failed: i64,
    /// Facts written across those runs.
    pub facts_persisted: i64,
}

/// What the worker measurably did: its runs, and the extractions still owed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ExtractionRunStats {
    /// Runs in the last 24 hours.
    pub last_24h: ExtractionRunCounts,
    /// Runs in the last 7 days.
    pub last_7d: ExtractionRunCounts,
    /// Unix milliseconds of the newest run still in the ledger.
    pub last_run_at_ms: Option<i64>,
    /// Unix milliseconds of the newest failed run still in the ledger.
    pub last_failure_at_ms: Option<i64>,
    /// Extractions owed for longer than a run's lease.
    pub backlog: i64,
    /// Unix milliseconds when the oldest of those was recorded.
    pub oldest_backlog_at_ms: Option<i64>,
}

/// The ledger of extractions owed, and of the runs that served them.
#[async_trait]
pub trait MemoryExtractionJobRepository: Send + Sync {
    /// Record an extraction before its spawn, unleased.
    async fn record_extraction_job(&self, row: &MemoryExtractionJobRow) -> AppResult<()>;

    /// Atomically lease up to `claim.limit` rows whose lease has lapsed, that
    /// were recorded before `now_ms - queued_older_than_ms`, and whose
    /// attempts are below the cap — oldest first, incrementing `attempts`.
    /// The returned rows carry the incremented count.
    async fn claim_stale_extraction_jobs(
        &self,
        claim: ExtractionJobClaim,
    ) -> AppResult<Vec<MemoryExtractionJobRow>>;

    /// Extend a running job's lease to `leased_until_ms`. `false` when the row
    /// is gone or another claim holds it; the caller stops renewing.
    async fn renew_extraction_job_lease(
        &self,
        lease: &JobLease,
        leased_until_ms: i64,
    ) -> AppResult<bool>;

    /// The extraction landed: delete the row, if this lease still holds it.
    /// `false` when another claim took it, whose run finishes it instead.
    async fn finish_extraction_job(&self, lease: &JobLease) -> AppResult<bool>;

    /// Delete every row at or past the attempt cap whose lease has lapsed:
    /// the run that was to finish it died too, and no claim will take it
    /// again, so the row would stand forever. Returns how many were removed.
    async fn reap_exhausted_extraction_jobs(
        &self,
        now_ms: i64,
        max_attempts: i64,
    ) -> AppResult<u64>;

    /// Record how one extraction run ended.
    async fn record_extraction_run(&self, run: &ExtractionRunRecord) -> AppResult<()>;

    /// Delete the run rows that ended before `before_ms`. Returns how many
    /// were removed.
    async fn prune_extraction_runs(&self, before_ms: i64) -> AppResult<u64>;

    /// The runs and backlog of `scope`: one tenant, or every tenant on the
    /// super-admin path of the memory worker screen. A job counts as backlog
    /// once it was recorded at or before `owed_since_ms`.
    async fn extraction_run_stats(
        &self,
        scope: AdminScope,
        now_ms: i64,
        owed_since_ms: i64,
    ) -> AppResult<ExtractionRunStats>;
}

const DAY_MS: i64 = 86_400_000;

pub(crate) const RECORD_EXTRACTION_RUN_SQL: &str = "INSERT INTO memory_extraction_runs \
     (id, tenant_id, outcome, facts_persisted, finished_at_ms) VALUES ($1, $2, $3, $4, $5)";

pub(crate) const PRUNE_EXTRACTION_RUNS_SQL: &str =
    "DELETE FROM memory_extraction_runs WHERE finished_at_ms < $1";

/// One row of run counters: `$1` opens the 24-hour window, `$2` the 7-day one,
/// and `$3` is the tenant, every tenant when it binds `NULL`.
pub(crate) const EXTRACTION_RUN_STATS_SQL: &str = "SELECT \
     CAST(COALESCE(SUM(CASE WHEN finished_at_ms >= $1 AND outcome = 'persisted' THEN 1 ELSE 0 END), 0) AS BIGINT) AS persisted_24h, \
     CAST(COALESCE(SUM(CASE WHEN finished_at_ms >= $1 AND outcome = 'empty' THEN 1 ELSE 0 END), 0) AS BIGINT) AS empty_24h, \
     CAST(COALESCE(SUM(CASE WHEN finished_at_ms >= $1 AND outcome = 'failed' THEN 1 ELSE 0 END), 0) AS BIGINT) AS failed_24h, \
     CAST(COALESCE(SUM(CASE WHEN finished_at_ms >= $1 THEN facts_persisted ELSE 0 END), 0) AS BIGINT) AS facts_24h, \
     CAST(COALESCE(SUM(CASE WHEN finished_at_ms >= $2 AND outcome = 'persisted' THEN 1 ELSE 0 END), 0) AS BIGINT) AS persisted_7d, \
     CAST(COALESCE(SUM(CASE WHEN finished_at_ms >= $2 AND outcome = 'empty' THEN 1 ELSE 0 END), 0) AS BIGINT) AS empty_7d, \
     CAST(COALESCE(SUM(CASE WHEN finished_at_ms >= $2 AND outcome = 'failed' THEN 1 ELSE 0 END), 0) AS BIGINT) AS failed_7d, \
     CAST(COALESCE(SUM(CASE WHEN finished_at_ms >= $2 THEN facts_persisted ELSE 0 END), 0) AS BIGINT) AS facts_7d, \
     MAX(finished_at_ms) AS last_run_at_ms, \
     MAX(CASE WHEN outcome = 'failed' THEN finished_at_ms END) AS last_failure_at_ms \
     FROM memory_extraction_runs \
     WHERE ($3 IS NULL OR tenant_id = $3)";

/// The jobs owed since `$1` of tenant `$2`, every tenant when it binds `NULL`.
pub(crate) const EXTRACTION_BACKLOG_SQL: &str =
    "SELECT CAST(COUNT(*) AS BIGINT) AS backlog, MIN(created_at_ms) AS oldest \
     FROM memory_extraction_jobs WHERE created_at_ms <= $1 AND ($2 IS NULL OR tenant_id = $2)";

/// The bounds of the two windows [`EXTRACTION_RUN_STATS_SQL`] counts over.
pub(crate) const fn run_windows(now_ms: i64) -> (i64, i64) {
    (
        now_ms.saturating_sub(DAY_MS),
        now_ms.saturating_sub(7 * DAY_MS),
    )
}

/// Decode the run counters and the backlog into one stats value.
pub(crate) fn extraction_run_stats_from_rows<R>(
    runs: &R,
    backlog: &R,
) -> AppResult<ExtractionRunStats>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let count = |row: &R, column: &str| -> AppResult<i64> {
        row.try_get(column)
            .map_err(|e| AppError::database(format!("extraction run stats {column}: {e}")))
    };
    let newest = |row: &R, column: &str| -> AppResult<Option<i64>> {
        row.try_get(column)
            .map_err(|e| AppError::database(format!("extraction run stats {column}: {e}")))
    };
    Ok(ExtractionRunStats {
        last_24h: ExtractionRunCounts {
            persisted: count(runs, "persisted_24h")?,
            empty: count(runs, "empty_24h")?,
            failed: count(runs, "failed_24h")?,
            facts_persisted: count(runs, "facts_24h")?,
        },
        last_7d: ExtractionRunCounts {
            persisted: count(runs, "persisted_7d")?,
            empty: count(runs, "empty_7d")?,
            failed: count(runs, "failed_7d")?,
            facts_persisted: count(runs, "facts_7d")?,
        },
        last_run_at_ms: newest(runs, "last_run_at_ms")?,
        last_failure_at_ms: newest(runs, "last_failure_at_ms")?,
        backlog: count(backlog, "backlog")?,
        oldest_backlog_at_ms: newest(backlog, "oldest")?,
    })
}

pub(crate) const RECORD_EXTRACTION_JOB_SQL: &str = "INSERT INTO memory_extraction_jobs \
     (id, tenant_id, usage_tenant_id, payload, created_at_ms, leased_until_ms, attempts, \
      lease_token) \
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8)";

/// The claim: one statement, so the lease and the attempt bump land
/// together and the subquery picks the rows under the same lock. The
/// backend's lock clause — `FOR UPDATE SKIP LOCKED` on `PostgreSQL`, nothing
/// on `SQLite` — is spliced in by the backend shell.
macro_rules! claim_extraction_jobs_sql {
    ($lock:literal) => {
        concat!(
            "UPDATE memory_extraction_jobs \
             SET leased_until_ms = $1, attempts = attempts + 1, lease_token = $6 \
             WHERE id IN ( \
                 SELECT j.id FROM memory_extraction_jobs j \
                 WHERE j.leased_until_ms <= $2 \
                   AND j.created_at_ms < $3 \
                   AND j.attempts < $4 \
                 ORDER BY j.created_at_ms ASC \
                 LIMIT $5 ",
            $lock,
            ") \
             RETURNING id, tenant_id, usage_tenant_id, payload, created_at_ms, leased_until_ms, \
             attempts, lease_token"
        )
    };
}
pub(crate) use claim_extraction_jobs_sql;

pub(crate) const RENEW_EXTRACTION_JOB_LEASE_SQL: &str =
    "UPDATE memory_extraction_jobs SET leased_until_ms = $1 \
     WHERE id = $2 AND lease_token = $3";

pub(crate) const FINISH_EXTRACTION_JOB_SQL: &str =
    "DELETE FROM memory_extraction_jobs WHERE id = $1 AND lease_token = $2";

pub(crate) const REAP_EXHAUSTED_EXTRACTION_JOBS_SQL: &str =
    "DELETE FROM memory_extraction_jobs WHERE attempts >= $1 AND leased_until_ms <= $2";

pub(crate) fn extraction_job_from_row<R>(row: &R) -> AppResult<MemoryExtractionJobRow>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    TenantId: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column = |name: &str, e: sqlx::Error| {
        AppError::database(format!("memory_extraction_jobs {name}: {e}"))
    };
    Ok(MemoryExtractionJobRow {
        id: row.try_get("id").map_err(|e| column("id", e))?,
        tenant_id: row
            .try_get("tenant_id")
            .map_err(|e| column("tenant_id", e))?,
        usage_tenant_id: row
            .try_get("usage_tenant_id")
            .map_err(|e| column("usage_tenant_id", e))?,
        payload: row.try_get("payload").map_err(|e| column("payload", e))?,
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
    })
}

/// Emit the whole [`MemoryExtractionJobRepository`] implementation for one
/// backend type. The body is written once here; each backend's shell invokes
/// it with its own type and its lock clause for [`claim_extraction_jobs_sql!`],
/// and sqlx resolves the driver from `self.pool()`.
///
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
macro_rules! impl_memory_extraction_job_repository {
    ($ty:ty, $lock:literal) => {
        #[async_trait::async_trait]
        impl MemoryExtractionJobRepository for $ty {
            async fn record_extraction_job(&self, row: &MemoryExtractionJobRow) -> AppResult<()> {
                sqlx::query(RECORD_EXTRACTION_JOB_SQL)
                    .bind(&row.id)
                    .bind(row.tenant_id)
                    .bind(row.usage_tenant_id)
                    .bind(&row.payload)
                    .bind(row.created_at_ms)
                    .bind(row.leased_until_ms)
                    .bind(row.attempts)
                    .bind(&row.lease_token)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record extraction job: {e}"))
                    })?;
                Ok(())
            }

            async fn claim_stale_extraction_jobs(
                &self,
                claim: ExtractionJobClaim,
            ) -> AppResult<Vec<MemoryExtractionJobRow>> {
                let rows = sqlx::query(claim_extraction_jobs_sql!($lock))
                    .bind(claim.now_ms.saturating_add(claim.lease_ms))
                    .bind(claim.now_ms)
                    .bind(claim.now_ms.saturating_sub(claim.queued_older_than_ms))
                    .bind(claim.max_attempts)
                    .bind(claim.limit)
                    .bind(uuid::Uuid::new_v4().to_string())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to claim extraction jobs: {e}"))
                    })?;
                rows.iter().map(extraction_job_from_row).collect()
            }

            async fn renew_extraction_job_lease(
                &self,
                lease: &JobLease,
                leased_until_ms: i64,
            ) -> AppResult<bool> {
                let result = sqlx::query(RENEW_EXTRACTION_JOB_LEASE_SQL)
                    .bind(leased_until_ms)
                    .bind(&lease.id)
                    .bind(&lease.token)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to renew extraction job lease: {e}"))
                    })?;
                Ok(result.rows_affected() == 1)
            }

            async fn finish_extraction_job(&self, lease: &JobLease) -> AppResult<bool> {
                let done = sqlx::query(FINISH_EXTRACTION_JOB_SQL)
                    .bind(&lease.id)
                    .bind(&lease.token)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to finish extraction job: {e}"))
                    })?;
                Ok(done.rows_affected() == 1)
            }

            async fn reap_exhausted_extraction_jobs(
                &self,
                now_ms: i64,
                max_attempts: i64,
            ) -> AppResult<u64> {
                let outcome = sqlx::query(REAP_EXHAUSTED_EXTRACTION_JOBS_SQL)
                    .bind(max_attempts)
                    .bind(now_ms)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to reap exhausted extraction jobs: {e}"))
                    })?;
                Ok(outcome.rows_affected())
            }

            async fn record_extraction_run(&self, run: &ExtractionRunRecord) -> AppResult<()> {
                sqlx::query(RECORD_EXTRACTION_RUN_SQL)
                    .bind(uuid::Uuid::new_v4().to_string())
                    .bind(run.tenant_id)
                    .bind(run.outcome.as_str())
                    .bind(run.facts_persisted)
                    .bind(run.finished_at_ms)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record extraction run: {e}"))
                    })?;
                Ok(())
            }

            async fn prune_extraction_runs(&self, before_ms: i64) -> AppResult<u64> {
                let outcome = sqlx::query(PRUNE_EXTRACTION_RUNS_SQL)
                    .bind(before_ms)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to prune extraction runs: {e}"))
                    })?;
                Ok(outcome.rows_affected())
            }

            async fn extraction_run_stats(
                &self,
                scope: AdminScope,
                now_ms: i64,
                owed_since_ms: i64,
            ) -> AppResult<ExtractionRunStats> {
                let tenant_id = scope.tenant_filter();
                let (since_24h, since_7d) = run_windows(now_ms);
                let read_failed =
                    |e| AppError::database(format!("Failed to read extraction runs: {e}"));
                let runs = sqlx::query(EXTRACTION_RUN_STATS_SQL)
                    .bind(since_24h)
                    .bind(since_7d)
                    .bind(tenant_id)
                    .fetch_one(self.pool())
                    .await
                    .map_err(read_failed)?;
                let backlog = sqlx::query(EXTRACTION_BACKLOG_SQL)
                    .bind(owed_since_ms)
                    .bind(tenant_id)
                    .fetch_one(self.pool())
                    .await
                    .map_err(read_failed)?;
                extraction_run_stats_from_rows(&runs, &backlog)
            }
        }
    };
}
pub(crate) use impl_memory_extraction_job_repository;
