// ABOUTME: SQLite-backed CommitmentRepository for athlete commitments and their swept verdicts
// ABOUTME: Conditional status transitions keep racing sweeps idempotent; mirrors backends/postgres/commitments.rs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_memory::commitments::Commitment;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use tracing::warn;

use crate::database::Database;
use crate::repositories::commitments::{
    commitment_from_row, CommitmentRepository, CommitmentRow, SweptVerdict, CANCEL_COMMITMENT_SQL,
    DUE_COMMITMENTS_SQL, EXPIRE_COMMITMENT_SQL, INSERT_COMMITMENT_SQL, LAST_REPORT_SQL,
    LIST_OPEN_COMMITMENTS_SQL, MARK_REPORTED_SQL, RECORD_VERDICT_SQL, UNREPORTED_COMMITMENTS_SQL,
};

/// Extract a [`CommitmentRow`] from a `SQLite` row via `try_get` only — keeps
/// corrupt-row handling symmetric with the Postgres impl. `SQLite` type affinity
/// can let a non-integer value land in an `INTEGER` column, so a column surprise
/// has to be a recoverable error the caller skips, never a panic.
fn sqlite_commitment_row(r: &SqliteRow) -> AppResult<CommitmentRow> {
    let col =
        |name: &str, e: sqlx::Error| AppError::database(format!("commitment col {name}: {e}"));
    Ok(CommitmentRow {
        id: r.try_get("id").map_err(|e| col("id", e))?,
        tenant_id: r.try_get("tenant_id").map_err(|e| col("tenant_id", e))?,
        user_id: r.try_get("user_id").map_err(|e| col("user_id", e))?,
        coach_id: r.try_get("coach_id").map_err(|e| col("coach_id", e))?,
        conversation_id: r
            .try_get("conversation_id")
            .map_err(|e| col("conversation_id", e))?,
        statement: r.try_get("statement").map_err(|e| col("statement", e))?,
        sport: r.try_get("sport").map_err(|e| col("sport", e))?,
        target_sessions: r
            .try_get("target_sessions")
            .map_err(|e| col("target_sessions", e))?,
        window_start: r
            .try_get("window_start")
            .map_err(|e| col("window_start", e))?,
        window_end: r.try_get("window_end").map_err(|e| col("window_end", e))?,
        status: r.try_get("status").map_err(|e| col("status", e))?,
        outcome: r.try_get("outcome").map_err(|e| col("outcome", e))?,
        completed_sessions: r
            .try_get("completed_sessions")
            .map_err(|e| col("completed_sessions", e))?,
        swept_at: r.try_get("swept_at").map_err(|e| col("swept_at", e))?,
        reported_at: r
            .try_get("reported_at")
            .map_err(|e| col("reported_at", e))?,
        created_at: r.try_get("created_at").map_err(|e| col("created_at", e))?,
        updated_at: r.try_get("updated_at").map_err(|e| col("updated_at", e))?,
    })
}

/// Map a fetched row set into commitments, skipping (with a warning) any row
/// whose columns cannot be read. One corrupt row must not blind the sweeper to
/// every other athlete's commitments.
fn collect_commitments(rows: &[SqliteRow]) -> Vec<Commitment> {
    rows.iter()
        .filter_map(|r| {
            sqlite_commitment_row(r)
                .and_then(commitment_from_row)
                .map_err(|e| warn!(error = %e, "skipping corrupt commitment row"))
                .ok()
        })
        .collect()
}

#[async_trait]
impl CommitmentRepository for Database {
    async fn insert_commitment(&self, commitment: &Commitment) -> AppResult<bool> {
        let result = sqlx::query(INSERT_COMMITMENT_SQL)
            .bind(&commitment.id)
            .bind(&commitment.tenant_id)
            .bind(&commitment.user_id)
            .bind(commitment.coach_id.as_deref().unwrap_or(""))
            .bind(commitment.conversation_id.as_deref().unwrap_or(""))
            .bind(&commitment.statement)
            .bind(commitment.sport.as_deref().unwrap_or(""))
            .bind(i64::from(commitment.target_sessions))
            .bind(commitment.window_start.timestamp())
            .bind(commitment.window_end.timestamp())
            .bind(commitment.status.as_str())
            .bind(commitment.created_at.timestamp())
            .bind(commitment.updated_at.timestamp())
            .execute(self.pool())
            .await
            .map_err(|e| AppError::database(format!("insert commitment: {e}")))?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_open_commitments(
        &self,
        tenant_id: &str,
        user_id: &str,
        limit: i64,
    ) -> AppResult<Vec<Commitment>> {
        let rows = sqlx::query(LIST_OPEN_COMMITMENTS_SQL)
            .bind(tenant_id)
            .bind(user_id)
            .bind(limit)
            .fetch_all(self.pool())
            .await
            .map_err(|e| AppError::database(format!("list open commitments: {e}")))?;
        Ok(collect_commitments(&rows))
    }

    async fn due_commitments(&self, now_epoch: i64, limit: i64) -> AppResult<Vec<Commitment>> {
        let rows = sqlx::query(DUE_COMMITMENTS_SQL)
            .bind(now_epoch)
            .bind(limit)
            .fetch_all(self.pool())
            .await
            .map_err(|e| AppError::database(format!("due commitments: {e}")))?;
        Ok(collect_commitments(&rows))
    }

    async fn unreported_commitments(&self, limit: i64) -> AppResult<Vec<Commitment>> {
        let rows = sqlx::query(UNREPORTED_COMMITMENTS_SQL)
            .bind(limit)
            .fetch_all(self.pool())
            .await
            .map_err(|e| AppError::database(format!("unreported commitments: {e}")))?;
        Ok(collect_commitments(&rows))
    }

    async fn record_commitment_verdict(&self, verdict: &SweptVerdict<'_>) -> AppResult<bool> {
        let result = sqlx::query(RECORD_VERDICT_SQL)
            .bind(verdict.outcome.as_str())
            .bind(i64::from(verdict.completed_sessions))
            .bind(verdict.at.timestamp())
            .bind(verdict.commitment_id)
            .bind(verdict.tenant_id)
            .execute(self.pool())
            .await
            .map_err(|e| AppError::database(format!("record commitment verdict: {e}")))?;
        Ok(result.rows_affected() > 0)
    }

    async fn mark_commitment_reported(
        &self,
        tenant_id: &str,
        commitment_id: &str,
        at: DateTime<Utc>,
    ) -> AppResult<bool> {
        let result = sqlx::query(MARK_REPORTED_SQL)
            .bind(at.timestamp())
            .bind(commitment_id)
            .bind(tenant_id)
            .execute(self.pool())
            .await
            .map_err(|e| AppError::database(format!("mark commitment reported: {e}")))?;
        Ok(result.rows_affected() > 0)
    }

    async fn expire_commitment(&self, tenant_id: &str, commitment_id: &str) -> AppResult<bool> {
        let result = sqlx::query(EXPIRE_COMMITMENT_SQL)
            .bind(Utc::now().timestamp())
            .bind(commitment_id)
            .bind(tenant_id)
            .execute(self.pool())
            .await
            .map_err(|e| AppError::database(format!("expire commitment: {e}")))?;
        Ok(result.rows_affected() > 0)
    }

    async fn cancel_commitment(&self, tenant_id: &str, commitment_id: &str) -> AppResult<bool> {
        let result = sqlx::query(CANCEL_COMMITMENT_SQL)
            .bind(Utc::now().timestamp())
            .bind(commitment_id)
            .bind(tenant_id)
            .execute(self.pool())
            .await
            .map_err(|e| AppError::database(format!("cancel commitment: {e}")))?;
        Ok(result.rows_affected() > 0)
    }

    async fn last_commitment_report(
        &self,
        tenant_id: &str,
        user_id: &str,
    ) -> AppResult<Option<DateTime<Utc>>> {
        let row = sqlx::query(LAST_REPORT_SQL)
            .bind(tenant_id)
            .bind(user_id)
            .fetch_optional(self.pool())
            .await
            .map_err(|e| AppError::database(format!("last commitment report: {e}")))?;
        let epoch: Option<i64> = match row {
            Some(r) => r
                .try_get("last_reported")
                .map_err(|e| AppError::database(format!("commitment col last_reported: {e}")))?,
            None => None,
        };
        Ok(epoch.and_then(|s| DateTime::from_timestamp(s, 0)))
    }
}
