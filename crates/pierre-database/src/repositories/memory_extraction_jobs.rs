// ABOUTME: Repository trait for post-turn memory extractions still owed — recorded before the spawn, deleted on success
// ABOUTME: The SQL is written once here and both backends emit their impl from it; a lapsed lease is what the resume sweep claims

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;

/// One extraction owed: the request the turn produced, as JSON, plus who
/// it is for and how many times it has been attempted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryExtractionJobRow {
    /// Job id (UUID string), minted by the recorder.
    pub id: String,
    /// Tenant the facts are stamped under.
    pub tenant_id: TenantId,
    /// The extraction request, serialised by `pierre-services`.
    pub payload: String,
    /// Unix milliseconds when the turn recorded it.
    pub created_at_ms: i64,
    /// Unix milliseconds until which one runner holds it; `0` when free.
    pub leased_until_ms: i64,
    /// Runs started so far, including the one this claim represents.
    pub attempts: i64,
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

/// The ledger of extractions owed.
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

    /// The extraction landed: delete the row.
    async fn finish_extraction_job(&self, id: &str) -> AppResult<()>;

    /// Delete every row at or past the attempt cap whose lease has lapsed:
    /// the run that was to finish it died too, and no claim will take it
    /// again, so the row would stand forever. Returns how many were removed.
    async fn reap_exhausted_extraction_jobs(
        &self,
        now_ms: i64,
        max_attempts: i64,
    ) -> AppResult<u64>;
}

pub(crate) const RECORD_EXTRACTION_JOB_SQL: &str = "INSERT INTO memory_extraction_jobs \
     (id, tenant_id, payload, created_at_ms, leased_until_ms, attempts) \
     VALUES ($1, $2, $3, $4, $5, $6)";

/// The claim: one statement, so the lease and the attempt bump land
/// together and the subquery picks the rows under the same lock. The
/// backend's lock clause — `FOR UPDATE SKIP LOCKED` on `PostgreSQL`, nothing
/// on `SQLite` — is spliced in by the backend shell.
macro_rules! claim_extraction_jobs_sql {
    ($lock:literal) => {
        concat!(
            "UPDATE memory_extraction_jobs \
             SET leased_until_ms = $1, attempts = attempts + 1 \
             WHERE id IN ( \
                 SELECT j.id FROM memory_extraction_jobs j \
                 WHERE j.leased_until_ms <= $2 \
                   AND j.created_at_ms < $3 \
                   AND j.attempts < $4 \
                 ORDER BY j.created_at_ms ASC \
                 LIMIT $5 ",
            $lock,
            ") \
             RETURNING id, tenant_id, payload, created_at_ms, leased_until_ms, attempts"
        )
    };
}
pub(crate) use claim_extraction_jobs_sql;

pub(crate) const FINISH_EXTRACTION_JOB_SQL: &str =
    "DELETE FROM memory_extraction_jobs WHERE id = $1";

pub(crate) const REAP_EXHAUSTED_EXTRACTION_JOBS_SQL: &str =
    "DELETE FROM memory_extraction_jobs WHERE attempts >= $1 AND leased_until_ms <= $2";

pub(crate) fn extraction_job_from_row<R>(row: &R) -> AppResult<MemoryExtractionJobRow>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
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
        payload: row.try_get("payload").map_err(|e| column("payload", e))?,
        created_at_ms: row
            .try_get("created_at_ms")
            .map_err(|e| column("created_at_ms", e))?,
        leased_until_ms: row
            .try_get("leased_until_ms")
            .map_err(|e| column("leased_until_ms", e))?,
        attempts: row.try_get("attempts").map_err(|e| column("attempts", e))?,
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
                    .bind(&row.payload)
                    .bind(row.created_at_ms)
                    .bind(row.leased_until_ms)
                    .bind(row.attempts)
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
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to claim extraction jobs: {e}"))
                    })?;
                rows.iter().map(extraction_job_from_row).collect()
            }

            async fn finish_extraction_job(&self, id: &str) -> AppResult<()> {
                sqlx::query(FINISH_EXTRACTION_JOB_SQL)
                    .bind(id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to finish extraction job: {e}"))
                    })?;
                Ok(())
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
        }
    };
}
pub(crate) use impl_memory_extraction_job_repository;
