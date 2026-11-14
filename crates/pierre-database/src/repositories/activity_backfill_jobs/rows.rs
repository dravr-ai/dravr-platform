// ABOUTME: Decodes an activity_backfill_jobs row: the whole job a claim returns, or the progress columns alone
// ABOUTME: Shared by both backends' implementations, which name these helpers unqualified

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

use super::{ActivityBackfillJobRow, BackfillJobKind, JobProgress};

fn column_error(name: &str, e: &sqlx::Error) -> AppError {
    AppError::database(format!("activity_backfill_jobs {name}: {e}"))
}

/// The [`JobProgress`] columns of `row`.
pub(crate) fn job_progress_from_row<R>(row: &R) -> AppResult<JobProgress>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    Option<i64>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<f64>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    // The unit columns keep the names the history walk gave them, a month
    // each; a detail walk's unit is an activity ([`JobProgress`]).
    Ok(JobProgress {
        units_total: row
            .try_get("months_total")
            .map_err(|e| column_error("months_total", &e))?,
        units_done: row
            .try_get("months_done")
            .map_err(|e| column_error("months_done", &e))?,
        rows_landed: row
            .try_get("rows_landed")
            .map_err(|e| column_error("rows_landed", &e))?,
        secs_per_unit: row
            .try_get("secs_per_unit")
            .map_err(|e| column_error("secs_per_unit", &e))?,
    })
}

pub(crate) fn backfill_job_from_row<R>(row: &R) -> AppResult<ActivityBackfillJobRow>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<i64>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<f64>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    TenantId: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let user_id: String = row
        .try_get("user_id")
        .map_err(|e| column_error("user_id", &e))?;
    let kind: String = row.try_get("kind").map_err(|e| column_error("kind", &e))?;
    Ok(ActivityBackfillJobRow {
        id: row.try_get("id").map_err(|e| column_error("id", &e))?,
        tenant_id: row
            .try_get("tenant_id")
            .map_err(|e| column_error("tenant_id", &e))?,
        user_id: Uuid::parse_str(&user_id)
            .map_err(|e| AppError::database(format!("backfill job user_id is not a UUID: {e}")))?,
        provider: row
            .try_get("provider")
            .map_err(|e| column_error("provider", &e))?,
        kind: BackfillJobKind::parse(&kind)?,
        after_ts: row
            .try_get("after_ts")
            .map_err(|e| column_error("after_ts", &e))?,
        before_ts: row
            .try_get("before_ts")
            .map_err(|e| column_error("before_ts", &e))?,
        fetch_limit: row
            .try_get("fetch_limit")
            .map_err(|e| column_error("fetch_limit", &e))?,
        conversation_id: row
            .try_get("conversation_id")
            .map_err(|e| column_error("conversation_id", &e))?,
        created_at_ms: row
            .try_get("created_at_ms")
            .map_err(|e| column_error("created_at_ms", &e))?,
        leased_until_ms: row
            .try_get("leased_until_ms")
            .map_err(|e| column_error("leased_until_ms", &e))?,
        attempts: row
            .try_get("attempts")
            .map_err(|e| column_error("attempts", &e))?,
        lease_token: row
            .try_get("lease_token")
            .map_err(|e| column_error("lease_token", &e))?,
        progress: job_progress_from_row(row)?,
        turn_id: row
            .try_get("turn_id")
            .map_err(|e| column_error("turn_id", &e))?,
        asked_message_id: row
            .try_get("asked_message_id")
            .map_err(|e| column_error("asked_message_id", &e))?,
        walk_backend: row
            .try_get("walk_backend")
            .map_err(|e| column_error("walk_backend", &e))?,
        walk_sport: row
            .try_get("walk_sport")
            .map_err(|e| column_error("walk_sport", &e))?,
    })
}
