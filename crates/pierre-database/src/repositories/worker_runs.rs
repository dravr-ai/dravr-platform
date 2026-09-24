// ABOUTME: Repository trait for the periodic-worker ledger — when each worker last ran and who holds its tick
// ABOUTME: The SQL is written once here and both backends emit their impl from it; pierre-services::periodic is the only caller

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};

/// What one worker's ledger row says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerRun {
    /// Unix milliseconds of the last tick that finished, `0` when none has.
    pub last_run_at_ms: i64,
    /// Unix milliseconds until which some instance holds the current tick,
    /// `0` when nobody does.
    pub leased_until_ms: i64,
}

/// The ledger every periodic worker consults before it ticks.
///
/// The row is global: workers are process-wide sweeps with no tenant, so
/// no statement here carries a `tenant_id`. `name` is the worker's display
/// name, which is already the key every log line carries.
#[async_trait]
pub trait WorkerRunRepository: Send + Sync {
    /// The worker's row, or `None` when it has never been claimed.
    async fn get_worker_run(&self, name: &str) -> AppResult<Option<WorkerRun>>;

    /// Take the worker's next tick, if it is due and nobody holds it.
    ///
    /// Due means `last_run_at_ms + period_ms <= now_ms`; held means
    /// `leased_until_ms > now_ms`. On success the lease is set to
    /// `now_ms + lease_ms` and `true` comes back; a worker that is not due,
    /// or whose tick another instance already took, gets `false`. The check
    /// and the write are one statement, so two instances asking at once get
    /// one `true` between them.
    async fn claim_worker_run(
        &self,
        name: &str,
        period_ms: i64,
        now_ms: i64,
        lease_ms: i64,
    ) -> AppResult<bool>;

    /// Record that the tick finished: `last_run_at_ms = now_ms`, lease released.
    async fn finish_worker_run(&self, name: &str, now_ms: i64) -> AppResult<()>;

    /// Record that the tick failed: hold the worker until `until_ms` and leave
    /// `last_run_at_ms` alone, so the retry waits out that hold — on this
    /// instance or any other — instead of firing on the next loop turn.
    async fn defer_worker_run(&self, name: &str, until_ms: i64) -> AppResult<()>;
}

pub(crate) const GET_WORKER_RUN_SQL: &str =
    "SELECT last_run_at_ms, leased_until_ms FROM worker_runs WHERE name = $1";

/// A missing row inserts (the worker has never run, so it is due); an
/// existing row updates only when the WHERE holds, and both drivers report
/// zero rows changed when it does not — that zero is the losing claim.
pub(crate) const CLAIM_WORKER_RUN_SQL: &str =
    "INSERT INTO worker_runs (name, last_run_at_ms, leased_until_ms) \
     VALUES ($1, 0, $2) \
     ON CONFLICT(name) DO UPDATE SET leased_until_ms = excluded.leased_until_ms \
     WHERE worker_runs.last_run_at_ms + $3 <= $4 AND worker_runs.leased_until_ms <= $4";

/// An upsert, so a finish recorded for a worker that was never claimed (a
/// seeded schedule, a ledger cleared underneath a running worker) still
/// lands instead of updating nothing.
pub(crate) const FINISH_WORKER_RUN_SQL: &str =
    "INSERT INTO worker_runs (name, last_run_at_ms, leased_until_ms) VALUES ($1, $2, 0) \
     ON CONFLICT(name) DO UPDATE SET last_run_at_ms = excluded.last_run_at_ms, \
     leased_until_ms = 0";

pub(crate) const DEFER_WORKER_RUN_SQL: &str =
    "INSERT INTO worker_runs (name, last_run_at_ms, leased_until_ms) VALUES ($1, 0, $2) \
     ON CONFLICT(name) DO UPDATE SET leased_until_ms = excluded.leased_until_ms";

pub(crate) fn worker_run_from_row<R>(row: &R) -> AppResult<WorkerRun>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    Ok(WorkerRun {
        last_run_at_ms: row
            .try_get("last_run_at_ms")
            .map_err(|e| AppError::database(format!("worker_runs last_run_at_ms: {e}")))?,
        leased_until_ms: row
            .try_get("leased_until_ms")
            .map_err(|e| AppError::database(format!("worker_runs leased_until_ms: {e}")))?,
    })
}

/// Emit the whole [`WorkerRunRepository`] implementation for one backend
/// type. The body is written once here; each backend's shell invokes it
/// with its own type, and sqlx resolves the driver from `self.pool()`.
///
/// The body names its consts, helpers and types unqualified, so the
/// invoking shell must `use` every one of them.
macro_rules! impl_worker_run_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl WorkerRunRepository for $ty {
            async fn get_worker_run(&self, name: &str) -> AppResult<Option<WorkerRun>> {
                let row = sqlx::query(GET_WORKER_RUN_SQL)
                    .bind(name)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to read worker_runs: {e}")))?;
                row.map(|r| worker_run_from_row(&r)).transpose()
            }

            async fn claim_worker_run(
                &self,
                name: &str,
                period_ms: i64,
                now_ms: i64,
                lease_ms: i64,
            ) -> AppResult<bool> {
                let result = sqlx::query(CLAIM_WORKER_RUN_SQL)
                    .bind(name)
                    .bind(now_ms.saturating_add(lease_ms))
                    .bind(period_ms)
                    .bind(now_ms)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to claim worker run: {e}")))?;
                Ok(result.rows_affected() == 1)
            }

            async fn finish_worker_run(&self, name: &str, now_ms: i64) -> AppResult<()> {
                sqlx::query(FINISH_WORKER_RUN_SQL)
                    .bind(name)
                    .bind(now_ms)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to finish worker run: {e}")))?;
                Ok(())
            }

            async fn defer_worker_run(&self, name: &str, until_ms: i64) -> AppResult<()> {
                sqlx::query(DEFER_WORKER_RUN_SQL)
                    .bind(name)
                    .bind(until_ms)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to defer worker run: {e}")))?;
                Ok(())
            }
        }
    };
}
pub(crate) use impl_worker_run_repository;
