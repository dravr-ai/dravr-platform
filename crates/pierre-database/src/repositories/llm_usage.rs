// ABOUTME: Shared statements, row decode and body for the llm_usage table both backends serve
// ABOUTME: One SQL text per operation; each backend shell supplies the turn_id codec and its spelling of a row's UTC day
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! LLM usage, written once.
//!
//! An `llm_usage` row is one LLM call: its tokens, cost and the tools it
//! invoked, attributed to a tenant, a user and a conversation turn. Every
//! id but `turn_id` is `TEXT` on both backends and binds as the text the
//! caller holds; `turn_id` is a `uuid` column on Postgres and `TEXT` on
//! `SQLite`, so the shell hands the body its [`uuid_columns`](super::uuid_columns)
//! codec. `created_at` is `TIMESTAMPTZ` on Postgres and RFC 3339 text on
//! `SQLite`; it binds as [`DateTime<Utc>`] on both (sqlx-sqlite writes the
//! bytes `to_rfc3339()` produces), reads back as one, and the record renders
//! it `to_rfc3339()` — the form [`insert_llm_usage`] itself returns, so a row
//! reads back as it was returned at insert. `call_sequence` is `INTEGER` on
//! Postgres, so it binds and reads as `i32` on both.
//!
//! Two things differ per engine. Sums and averages: Postgres sums a `BIGINT`
//! column as `NUMERIC` and averages an integer one likewise, which sqlx will
//! not decode as `i64` or `f64`, so every aggregate is cast in the statement
//! — `CAST(.. AS BIGINT)` and `CAST(.. AS DOUBLE PRECISION)` are spelled the
//! same on both. And the UTC calendar day of a row, which the daily series
//! groups by: `DATE(created_at)` on `SQLite`, whose stored text is already
//! UTC, and `TO_CHAR(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD')` on
//! Postgres; the shell passes that spelling as the `day` literal.
//!
//! The `since` a caller hands the aggregate reads is parsed here, once, and
//! bound as a [`DateTime<Utc>`] on both — see [`since_bound`].
//!
//! [`insert_llm_usage`]: super::LlmUsageRepository::insert_llm_usage

use std::fmt::Display;

use chrono::{DateTime, NaiveDate, NaiveTime, SubsecRound, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::usage::{LlmUsageAggregateRow, LlmUsageDailyRow};

/// Allowed grouping dimensions for LLM consumption aggregation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmUsageGroupBy {
    /// Group by provider name
    Provider,
    /// Group by model identifier
    Model,
    /// Group by call type
    CallType,
}

impl LlmUsageGroupBy {
    /// Parse from query string parameter
    #[must_use]
    pub fn from_str_param(s: &str) -> Option<Self> {
        match s {
            "provider" => Some(Self::Provider),
            "model" => Some(Self::Model),
            "call_type" => Some(Self::CallType),
            _ => None,
        }
    }
}

/// The lower bound of an aggregate read, as the instant it names.
///
/// Callers spell it two ways: RFC 3339 (`to_rfc3339()` from the admin and
/// web-admin handlers) and a bare `YYYY-MM-DD` (the dashboard's window
/// start), which is midnight UTC of that day. Bound as a [`DateTime<Utc>`]
/// the compare is an instant on both engines; handed to the statement as
/// text it was a `::timestamptz` cast on Postgres and a lexical compare on
/// `SQLite`, where a `since` in any other shape matched everything or
/// nothing without a word.
///
/// # Errors
/// Returns an invalid-input error when the text is neither form.
pub fn since_bound(since: &str) -> AppResult<DateTime<Utc>> {
    if let Ok(instant) = DateTime::parse_from_rfc3339(since) {
        return Ok(instant.with_timezone(&Utc));
    }
    let day = NaiveDate::parse_from_str(since, "%Y-%m-%d").map_err(|e| {
        AppError::invalid_input(format!(
            "since must be RFC 3339 or YYYY-MM-DD, got {since:?}: {e}"
        ))
    })?;
    Ok(DateTime::<Utc>::from_naive_utc_and_offset(
        day.and_time(NaiveTime::MIN),
        Utc,
    ))
}

/// The columns [`impl_llm_usage_repository!`]'s record parser reads, in
/// the order the insert writes them.
macro_rules! llm_usage_columns {
    () => {
        "id, tenant_id, user_id, conversation_id, turn_id, provider, model, \
         prompt_tokens, completion_tokens, total_tokens, cached_tokens, cached_write_tokens, \
         reasoning_tokens, call_type, tool_calls_count, tools_called, execution_time_ms, \
         cost_usd, call_sequence, created_at"
    };
}

/// One LLM call.
pub(crate) const INSERT_LLM_USAGE_SQL: &str = concat!(
    "INSERT INTO llm_usage (",
    llm_usage_columns!(),
    ") VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20)"
);

/// The token sums every aggregate projects. Postgres sums a `BIGINT` as
/// `NUMERIC`; the cast reads as `i64` on both.
macro_rules! llm_usage_token_sums {
    () => {
        "CAST(COALESCE(SUM(prompt_tokens), 0) AS BIGINT) AS prompt_tokens, \
         CAST(COALESCE(SUM(completion_tokens), 0) AS BIGINT) AS completion_tokens, \
         CAST(COALESCE(SUM(cached_tokens), 0) AS BIGINT) AS cached_tokens, \
         CAST(COALESCE(SUM(cached_write_tokens), 0) AS BIGINT) AS cached_write_tokens, \
         CAST(COALESCE(SUM(reasoning_tokens), 0) AS BIGINT) AS reasoning_tokens, \
         CAST(COUNT(*) AS BIGINT) AS calls"
    };
}
pub(crate) use llm_usage_token_sums;

/// Totals per `(provider, model, call_type)` for one scope since `$2`,
/// heaviest first; turn-summary calls carry no tokens and are left out so
/// they do not inflate `calls`. `$scope` is the column the scope binds to:
/// `tenant_id` or `user_id`.
macro_rules! llm_usage_aggregates_sql {
    ($scope:literal) => {
        concat!(
            "SELECT provider, model, call_type, \
                    CAST(COALESCE(SUM(total_tokens), 0) AS BIGINT) AS total_tokens, ",
            llm_usage_token_sums!(),
            " FROM llm_usage \
              WHERE ",
            $scope,
            " = $1 AND created_at >= $2 AND call_type != $3 \
              GROUP BY provider, model, call_type \
              ORDER BY total_tokens DESC"
        )
    };
}

/// A tenant's totals per `(provider, model, call_type)` since `$2`.
pub(crate) const LLM_USAGE_AGGREGATES_SQL: &str = llm_usage_aggregates_sql!("tenant_id");

/// A user's totals per `(provider, model, call_type)` since `$2`.
pub(crate) const LLM_USAGE_AGGREGATES_BY_USER_SQL: &str = llm_usage_aggregates_sql!("user_id");

/// Totals per UTC calendar day for one scope since `$2`, oldest first, with
/// the same turn-summary exclusion as the aggregates. `$day` is the
/// engine's spelling of a row's UTC day as `YYYY-MM-DD` text; `$scope` is
/// `tenant_id` or `user_id`. Postgres averages an integer column as
/// `NUMERIC`, so the average is cast to a double on both.
macro_rules! llm_usage_daily_series_sql {
    ($day:literal, $scope:literal) => {
        concat!(
            "SELECT ",
            $day,
            " AS date, \
                    CAST(COALESCE(SUM(total_tokens), 0) AS BIGINT) AS tokens, ",
            llm_usage_token_sums!(),
            ", CAST(AVG(execution_time_ms) AS DOUBLE PRECISION) AS avg_exec_ms \
              FROM llm_usage \
              WHERE ",
            $scope,
            " = $1 AND created_at >= $2 AND call_type != $3 \
              GROUP BY ",
            $day,
            " ORDER BY date ASC"
        )
    };
}
pub(crate) use llm_usage_daily_series_sql;

/// The most recent `$1` calls across every tenant, newest first.
pub(crate) const RECENT_LLM_CALLS_SQL: &str = concat!(
    "SELECT ",
    llm_usage_columns!(),
    " FROM llm_usage ORDER BY created_at DESC LIMIT $1"
);

/// A tenant's calls since `$2` that invoked at least one tool, newest
/// first. Per-tool aggregation happens in the handler, so no JSON SQL.
pub(crate) const TENANT_TOOL_CALLS_SINCE_SQL: &str = concat!(
    "SELECT ",
    llm_usage_columns!(),
    " FROM llm_usage \
      WHERE tenant_id = $1 AND created_at >= $2 AND tools_called <> '[]' \
      ORDER BY created_at DESC"
);

/// Every call of one conversation turn, in the order the turn made them; a
/// row backfilled without a sequence sorts last.
pub(crate) const LLM_USAGE_BY_TURN_SQL: &str = concat!(
    "SELECT ",
    llm_usage_columns!(),
    " FROM llm_usage WHERE turn_id = $1 \
      ORDER BY call_sequence ASC NULLS LAST, created_at ASC"
);

/// How many calls landed since `$1`, across every tenant.
pub(crate) const COUNT_LLM_CALLS_SINCE_SQL: &str =
    "SELECT CAST(COUNT(*) AS BIGINT) AS calls FROM llm_usage WHERE created_at >= $1";

/// Calls and tokens since `$1`, across every tenant.
pub(crate) const SUM_LLM_USAGE_SINCE_SQL: &str = "SELECT CAST(COUNT(*) AS BIGINT) AS calls, \
            CAST(COALESCE(SUM(total_tokens), 0) AS BIGINT) AS total_tokens \
     FROM llm_usage WHERE created_at >= $1";

/// A tenant's spend over `[$2, $3)`.
pub(crate) const SUM_COST_USD_FOR_TENANT_PERIOD_SQL: &str =
    "SELECT COALESCE(SUM(cost_usd), 0.0) AS cost_usd \
     FROM llm_usage WHERE tenant_id = $1 AND created_at >= $2 AND created_at < $3";

/// The error a column that will not decode surfaces as, named after the
/// column so a corrupt row is locatable from the message.
pub(crate) fn llm_usage_column_error(col: &str, e: impl Display) -> AppError {
    AppError::database(format!("llm_usage column {col}: {e}"))
}

/// One row of [`LLM_USAGE_AGGREGATES_SQL`] or its per-user twin.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn aggregate_from_row<R>(row: &R) -> AppResult<LlmUsageAggregateRow>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let text = |col: &str| -> AppResult<String> {
        row.try_get(col).map_err(|e| llm_usage_column_error(col, e))
    };
    let count = |col: &str| -> AppResult<i64> {
        row.try_get(col).map_err(|e| llm_usage_column_error(col, e))
    };
    Ok(LlmUsageAggregateRow {
        provider: text("provider")?,
        model: text("model")?,
        call_type: text("call_type")?,
        total_tokens: count("total_tokens")?,
        prompt_tokens: count("prompt_tokens")?,
        completion_tokens: count("completion_tokens")?,
        cached_tokens: count("cached_tokens")?,
        cached_write_tokens: count("cached_write_tokens")?,
        reasoning_tokens: count("reasoning_tokens")?,
        calls: count("calls")?,
    })
}

/// One row of [`llm_usage_daily_series_sql!`]. A day whose calls all lack an
/// execution time averages as zero.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn daily_from_row<R>(row: &R) -> AppResult<LlmUsageDailyRow>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<f64>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let count = |col: &str| -> AppResult<i64> {
        row.try_get(col).map_err(|e| llm_usage_column_error(col, e))
    };
    let avg_exec_ms: Option<f64> = row
        .try_get("avg_exec_ms")
        .map_err(|e| llm_usage_column_error("avg_exec_ms", e))?;
    Ok(LlmUsageDailyRow {
        date: row
            .try_get("date")
            .map_err(|e| llm_usage_column_error("date", e))?,
        tokens: count("tokens")?,
        prompt_tokens: count("prompt_tokens")?,
        completion_tokens: count("completion_tokens")?,
        cached_tokens: count("cached_tokens")?,
        cached_write_tokens: count("cached_write_tokens")?,
        reasoning_tokens: count("reasoning_tokens")?,
        calls: count("calls")?,
        avg_execution_time_ms: avg_exec_ms.unwrap_or(0.0),
    })
}

/// The instant a usage row is stamped with: now, at the microsecond
/// precision `TIMESTAMPTZ` holds.
///
/// A `DateTime<Utc>` carries nanoseconds on Linux and Postgres keeps six
/// digits, so a record returned with the untruncated instant would not
/// match the row it reads back as; `SQLite` stores the text whole and is
/// truncated the same so both engines agree.
#[must_use]
pub fn recorded_at() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

/// A call's position in its turn as the `INTEGER` column holds it; a value
/// past `i32::MAX` saturates rather than refusing the usage row.
#[must_use]
pub fn call_sequence_column(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// Emit the whole [`LlmUsageRepository`](super::LlmUsageRepository)
/// implementation for one backend type.
///
/// The body is written once here; each backend's shell invokes it with its
/// own type, its driver's row type, its uuid codec from
/// [`super::uuid_columns`] for `turn_id`, and the one spelling only it
/// knows: a row's UTC calendar day as text (`day`). The record parser is
/// emitted inside the macro because the `turn_id` read is what differs per
/// driver; sqlx resolves the driver from `self.pool()` per expansion.
///
/// The body names its consts, helpers and types unqualified, so the
/// invoking shell must `use` every one of them.
macro_rules! impl_llm_usage_repository {
    ($ty:ty, $row:ty, $ids:ident, day = $day:literal) => {
        /// One [`llm_usage_columns!`] row as the record the API carries.
        fn llm_usage_record_from_row(row: &$row) -> AppResult<LlmUsageRecord> {
            let text = |col: &str| -> AppResult<String> {
                row.try_get(col).map_err(|e| llm_usage_column_error(col, e))
            };
            let count = |col: &str| -> AppResult<i64> {
                row.try_get(col).map_err(|e| llm_usage_column_error(col, e))
            };
            let created_at: DateTime<Utc> = row
                .try_get("created_at")
                .map_err(|e| llm_usage_column_error("created_at", e))?;
            let call_sequence: Option<i32> = row
                .try_get("call_sequence")
                .map_err(|e| llm_usage_column_error("call_sequence", e))?;
            Ok(LlmUsageRecord {
                id: text("id")?,
                tenant_id: text("tenant_id")?,
                user_id: text("user_id")?,
                conversation_id: row
                    .try_get("conversation_id")
                    .map_err(|e| llm_usage_column_error("conversation_id", e))?,
                turn_id: ConversationTurnId::from_uuid($ids::read(row, "turn_id")?),
                provider: text("provider")?,
                model: text("model")?,
                prompt_tokens: count("prompt_tokens")?,
                completion_tokens: count("completion_tokens")?,
                total_tokens: count("total_tokens")?,
                cached_tokens: count("cached_tokens")?,
                cached_write_tokens: count("cached_write_tokens")?,
                reasoning_tokens: count("reasoning_tokens")?,
                call_type: text("call_type")?,
                tool_calls_count: count("tool_calls_count")?,
                tools_called: text("tools_called")?,
                execution_time_ms: row
                    .try_get("execution_time_ms")
                    .map_err(|e| llm_usage_column_error("execution_time_ms", e))?,
                cost_usd: row
                    .try_get("cost_usd")
                    .map_err(|e| llm_usage_column_error("cost_usd", e))?,
                call_sequence: call_sequence.map(i64::from),
                created_at: created_at.to_rfc3339(),
            })
        }

        #[async_trait::async_trait]
        impl LlmUsageRepository for $ty {
            async fn insert_llm_usage(
                &self,
                params: &InsertLlmUsage<'_>,
            ) -> AppResult<LlmUsageRecord> {
                let id = Uuid::new_v4().to_string();
                let now = recorded_at();

                sqlx::query(INSERT_LLM_USAGE_SQL)
                    .bind(&id)
                    .bind(params.tenant_id)
                    .bind(params.user_id)
                    .bind(params.conversation_id)
                    .bind($ids::bind(params.turn_id.as_uuid()))
                    .bind(params.provider)
                    .bind(params.model)
                    .bind(params.prompt_tokens)
                    .bind(params.completion_tokens)
                    .bind(params.total_tokens)
                    .bind(params.cached_tokens)
                    .bind(params.cached_write_tokens)
                    .bind(params.reasoning_tokens)
                    .bind(params.call_type)
                    .bind(params.tool_calls_count)
                    .bind(params.tools_called)
                    .bind(params.execution_time_ms)
                    .bind(params.cost_usd)
                    .bind(params.call_sequence.map(call_sequence_column))
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to insert LLM usage: {e}")))?;

                Ok(LlmUsageRecord {
                    id,
                    tenant_id: params.tenant_id.to_owned(),
                    user_id: params.user_id.to_owned(),
                    conversation_id: params.conversation_id.map(ToOwned::to_owned),
                    turn_id: params.turn_id,
                    provider: params.provider.to_owned(),
                    model: params.model.to_owned(),
                    prompt_tokens: params.prompt_tokens,
                    completion_tokens: params.completion_tokens,
                    total_tokens: params.total_tokens,
                    cached_tokens: params.cached_tokens,
                    cached_write_tokens: params.cached_write_tokens,
                    reasoning_tokens: params.reasoning_tokens,
                    call_type: params.call_type.to_owned(),
                    tool_calls_count: params.tool_calls_count,
                    tools_called: params.tools_called.to_owned(),
                    execution_time_ms: params.execution_time_ms,
                    cost_usd: params.cost_usd,
                    call_sequence: params
                        .call_sequence
                        .map(|v| i64::from(call_sequence_column(v))),
                    created_at: now.to_rfc3339(),
                })
            }

            async fn get_llm_usage_aggregates(
                &self,
                tenant_id: &str,
                since: &str,
            ) -> AppResult<Vec<LlmUsageAggregateRow>> {
                sqlx::query(LLM_USAGE_AGGREGATES_SQL)
                    .bind(tenant_id)
                    .bind(since_bound(since)?)
                    .bind(TURN_SUMMARY_CALL_TYPE)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query LLM usage aggregates: {e}"))
                    })?
                    .iter()
                    .map(aggregate_from_row)
                    .collect()
            }

            async fn get_llm_usage_daily_series(
                &self,
                tenant_id: &str,
                since: &str,
            ) -> AppResult<Vec<LlmUsageDailyRow>> {
                sqlx::query(llm_usage_daily_series_sql!($day, "tenant_id"))
                    .bind(tenant_id)
                    .bind(since_bound(since)?)
                    .bind(TURN_SUMMARY_CALL_TYPE)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query LLM usage daily series: {e}"))
                    })?
                    .iter()
                    .map(daily_from_row)
                    .collect()
            }

            async fn get_llm_usage_aggregates_by_user(
                &self,
                user_id: &str,
                since: &str,
            ) -> AppResult<Vec<LlmUsageAggregateRow>> {
                sqlx::query(LLM_USAGE_AGGREGATES_BY_USER_SQL)
                    .bind(user_id)
                    .bind(since_bound(since)?)
                    .bind(TURN_SUMMARY_CALL_TYPE)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to query LLM usage aggregates by user: {e}"
                        ))
                    })?
                    .iter()
                    .map(aggregate_from_row)
                    .collect()
            }

            async fn get_llm_usage_daily_series_by_user(
                &self,
                user_id: &str,
                since: &str,
            ) -> AppResult<Vec<LlmUsageDailyRow>> {
                sqlx::query(llm_usage_daily_series_sql!($day, "user_id"))
                    .bind(user_id)
                    .bind(since_bound(since)?)
                    .bind(TURN_SUMMARY_CALL_TYPE)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to query LLM usage daily series by user: {e}"
                        ))
                    })?
                    .iter()
                    .map(daily_from_row)
                    .collect()
            }

            async fn get_recent_llm_calls_admin(
                &self,
                limit: i64,
            ) -> AppResult<Vec<LlmUsageRecord>> {
                sqlx::query(RECENT_LLM_CALLS_SQL)
                    .bind(limit)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query LLM usage rows: {e}"))
                    })?
                    .iter()
                    .map(llm_usage_record_from_row)
                    .collect()
            }

            async fn get_tenant_tool_calls_since(
                &self,
                tenant_id: TenantId,
                since: &str,
            ) -> AppResult<Vec<LlmUsageRecord>> {
                sqlx::query(TENANT_TOOL_CALLS_SINCE_SQL)
                    .bind(tenant_id.to_string())
                    .bind(since_bound(since)?)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query LLM usage rows: {e}"))
                    })?
                    .iter()
                    .map(llm_usage_record_from_row)
                    .collect()
            }

            async fn count_llm_calls_since(&self, since: &str) -> AppResult<i64> {
                let row = sqlx::query(COUNT_LLM_CALLS_SINCE_SQL)
                    .bind(since_bound(since)?)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to count LLM calls: {e}")))?;
                row.try_get("calls")
                    .map_err(|e| llm_usage_column_error("calls", e))
            }

            async fn sum_llm_usage_since(&self, since: &str) -> AppResult<(i64, i64)> {
                let row = sqlx::query(SUM_LLM_USAGE_SINCE_SQL)
                    .bind(since_bound(since)?)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to sum LLM usage: {e}")))?;
                Ok((
                    row.try_get("calls")
                        .map_err(|e| llm_usage_column_error("calls", e))?,
                    row.try_get("total_tokens")
                        .map_err(|e| llm_usage_column_error("total_tokens", e))?,
                ))
            }

            async fn find_llm_usage_by_turn_id(
                &self,
                turn_id: ConversationTurnId,
            ) -> AppResult<Vec<LlmUsageRecord>> {
                sqlx::query(LLM_USAGE_BY_TURN_SQL)
                    .bind($ids::bind(turn_id.as_uuid()))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to query LLM usage rows: {e}"))
                    })?
                    .iter()
                    .map(llm_usage_record_from_row)
                    .collect()
            }

            async fn sum_cost_usd_for_tenant_period(
                &self,
                tenant_id: TenantId,
                start: DateTime<Utc>,
                end: DateTime<Utc>,
            ) -> AppResult<f64> {
                let row = sqlx::query(SUM_COST_USD_FOR_TENANT_PERIOD_SQL)
                    .bind(tenant_id.to_string())
                    .bind(start)
                    .bind(end)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to sum cost_usd: {e}")))?;
                let total: Option<f64> = row
                    .try_get("cost_usd")
                    .map_err(|e| llm_usage_column_error("cost_usd", e))?;
                Ok(total.unwrap_or(0.0))
            }
        }
    };
}
pub(crate) use impl_llm_usage_repository;
