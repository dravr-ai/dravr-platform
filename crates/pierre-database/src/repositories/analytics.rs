// ABOUTME: Shared statements, row decode and body for API-key and JWT usage accounting both backends serve
// ABOUTME: One SQL text per operation; each backend shell supplies the uuid codec, the usage-id clause and the inet cast
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Usage accounting, written once.
//!
//! `api_key_usage` holds one row per API-key call and `jwt_usage` one per
//! JWT-authenticated request; the request log, the per-key window count,
//! the period stats and the top-tools analysis all read the first. Three
//! things differ per engine, and the shell passes each one:
//!
//! - **ids.** `jwt_usage.user_id` and `api_keys.user_id` are `uuid` on
//!   Postgres and `TEXT` on `SQLite`, so a user id binds through the
//!   [`uuid_columns`](super::uuid_columns) codec. `api_key_id` is `TEXT` on
//!   both.
//! - **the usage row's own id.** `SERIAL` on Postgres, where the insert
//!   says `DEFAULT` and the table mints it; `TEXT PRIMARY KEY` with no
//!   default on `SQLite`, where the insert mints a hex id in SQL. The
//!   spelling is the `usage_id` literal.
//! - **the client address.** `INET` on Postgres, which needs the `::inet`
//!   cast on the bound text; `TEXT` on `SQLite`. The cast is the `inet`
//!   literal, empty there.
//!
//! Timestamps bind and read as [`DateTime<Utc>`] on both (`TIMESTAMPTZ` on
//! Postgres, the RFC 3339 text `to_rfc3339()` produces on `SQLite`), so a
//! window compares as an instant and not as two differently spelled
//! strings — including the JWT month-to-date window, whose start is
//! computed here rather than by each engine's calendar function.
//! `api_key_usage.status_code` is `SMALLINT` on Postgres and binds as `i16`
//! on both; every count reads as `i64` and every average is cast to a
//! double in the statement, because Postgres averages an integer column as
//! `NUMERIC`, which sqlx will not decode as `f64`.

use std::fmt::{Display, Write};

use chrono::{DateTime, Datelike, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::RequestLog;

/// One API-key call. `$id` is the engine's spelling of the row's own id and
/// `$inet` the cast the address column needs; `method` has no value in the
/// model and is left to its NULL default.
macro_rules! record_api_key_usage_sql {
    ($id:literal, $inet:literal) => {
        concat!(
            "INSERT INTO api_key_usage (id, api_key_id, timestamp, endpoint, status_code, \
                 response_time_ms, request_size_bytes, response_size_bytes, ip_address, \
                 user_agent, error_message) \
             VALUES (",
            $id,
            ", $1, $2, $3, $4, $5, $6, $7, $8",
            $inet,
            ", $9, $10)"
        )
    };
}
pub(crate) use record_api_key_usage_sql;

/// One JWT-authenticated request; the id and address clauses are as for
/// [`record_api_key_usage_sql!`].
macro_rules! record_jwt_usage_sql {
    ($id:literal, $inet:literal) => {
        concat!(
            "INSERT INTO jwt_usage (id, user_id, timestamp, endpoint, method, status_code, \
                 response_time_ms, request_size_bytes, response_size_bytes, ip_address, user_agent) \
             VALUES (",
            $id,
            ", $1, $2, $3, $4, $5, $6, $7, $8, $9",
            $inet,
            ", $10)"
        )
    };
}
pub(crate) use record_jwt_usage_sql;

/// A key's calls after `$2`: its current usage against its own rate-limit
/// window.
pub(crate) const API_KEY_CURRENT_USAGE_SQL: &str =
    "SELECT COUNT(*) AS count FROM api_key_usage WHERE api_key_id = $1 AND timestamp > $2";

/// A key's totals over `[$5, $6]`. `$1..=$3` are the success band and the
/// failure floor: a 2xx is successful, 400 and up failed, so a 3xx is in
/// neither.
pub(crate) const API_KEY_STATS_SQL: &str = "SELECT COUNT(*) AS total_requests, \
            COUNT(CASE WHEN status_code >= $1 AND status_code <= $2 THEN 1 END) AS successful_requests, \
            COUNT(CASE WHEN status_code >= $3 THEN 1 END) AS failed_requests, \
            SUM(response_time_ms) AS total_response_time \
     FROM api_key_usage \
     WHERE api_key_id = $4 AND timestamp >= $5 AND timestamp <= $6";

/// A key's calls per tool over `[$4, $5]`, most called first, with the
/// success band `$1..=$2`.
pub(crate) const API_KEY_TOOL_USAGE_SQL: &str = "SELECT endpoint, COUNT(*) AS tool_count, \
            CAST(AVG(response_time_ms) AS DOUBLE PRECISION) AS avg_response_time, \
            COUNT(CASE WHEN status_code >= $1 AND status_code <= $2 THEN 1 END) AS success_count \
     FROM api_key_usage \
     WHERE api_key_id = $3 AND timestamp >= $4 AND timestamp <= $5 \
     GROUP BY endpoint \
     ORDER BY tool_count DESC";

/// A user's JWT requests since `$2`: the month-to-date count the tier's
/// monthly limit is enforced against.
pub(crate) const JWT_CURRENT_USAGE_SQL: &str =
    "SELECT COUNT(*) AS count FROM jwt_usage WHERE user_id = $1 AND timestamp >= $2";

/// A user's calls per tool over `[$2, $3]`, most called first, ten at most;
/// `$4` is the failure floor.
pub(crate) const TOP_TOOLS_SQL: &str = "SELECT aku.endpoint, COUNT(*) AS usage_count, \
            CAST(AVG(aku.response_time_ms) AS DOUBLE PRECISION) AS avg_response_time, \
            COUNT(CASE WHEN aku.status_code < $4 THEN 1 END) AS success_count, \
            COUNT(CASE WHEN aku.status_code >= $4 THEN 1 END) AS error_count \
     FROM api_key_usage aku \
     JOIN api_keys ak ON aku.api_key_id = ak.id \
     WHERE ak.user_id = $1 AND aku.timestamp BETWEEN $2 AND $3 \
     GROUP BY aku.endpoint \
     ORDER BY usage_count DESC \
     LIMIT 10";

/// The projection every request-log read starts from. `api_key_usage` is
/// the table the request path writes; `api_keys` is joined unconditionally
/// so the log carries the key's real name, and the join is total —
/// `api_key_usage.api_key_id` is NOT NULL and references `api_keys`. The
/// id is cast to text because it is a `SERIAL` on Postgres and text on
/// `SQLite`; the status is cast to an integer because it is a `SMALLINT`
/// on Postgres.
const REQUEST_LOGS_SELECT: &str = "SELECT CAST(u.id AS TEXT) AS id, u.timestamp, u.api_key_id, \
            k.name AS api_key_name, u.endpoint AS tool_name, \
            CAST(u.status_code AS INTEGER) AS status_code, u.response_time_ms, \
            u.error_message, u.request_size_bytes, u.response_size_bytes \
     FROM api_key_usage u \
     JOIN api_keys k ON u.api_key_id = k.id \
     WHERE 1=1";

/// The most rows one request-log read returns.
pub(crate) const REQUEST_LOGS_LIMIT: u16 = 1000;

/// Which filters a request-log read applies, in the bind order
/// [`impl_usage_repository!`]'s `get_request_logs` follows: user, key,
/// start, end, status prefix, tool substring.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RequestLogFilters {
    pub user: bool,
    pub api_key: bool,
    pub start: bool,
    pub end: bool,
    pub status: bool,
    pub tool: bool,
}

/// The request-log statement for one set of filters. The status filter is
/// a prefix over the status rendered as text, so `"5"` is every 5xx; the
/// tool filter is a substring compared in lower case on both sides, which
/// is what `SQLite`'s `LIKE` and Postgres's `ILIKE` each did alone.
///
/// # Errors
/// Returns an internal error if the clause cannot be written, which a
/// `String` never refuses.
pub(crate) fn request_logs_sql(filters: RequestLogFilters) -> AppResult<String> {
    let mut query = String::from(REQUEST_LOGS_SELECT);
    let mut bind_count = 0;
    for (present, clause, close) in [
        (filters.user, " AND k.user_id = $", ""),
        (filters.api_key, " AND u.api_key_id = $", ""),
        (filters.start, " AND u.timestamp >= $", ""),
        (filters.end, " AND u.timestamp <= $", ""),
        (
            filters.status,
            " AND CAST(u.status_code AS TEXT) LIKE $",
            "",
        ),
        (filters.tool, " AND LOWER(u.endpoint) LIKE LOWER($", ")"),
    ] {
        if present {
            bind_count += 1;
            write!(query, "{clause}{bind_count}{close}").map_err(|e| {
                AppError::internal(format!("Failed to write request log filter: {e}"))
            })?;
        }
    }
    write!(
        query,
        " ORDER BY u.timestamp DESC LIMIT {REQUEST_LOGS_LIMIT}"
    )
    .map_err(|e| AppError::internal(format!("Failed to write request log limit: {e}")))?;
    Ok(query)
}

/// The error a column that will not decode surfaces as, named after the
/// column so a corrupt row is locatable from the message.
pub(crate) fn usage_column_error(col: &str, e: impl Display) -> AppError {
    AppError::database(format!("usage column {col}: {e}"))
}

/// A count the engine returns as `i64`, as the `u32` the model carries.
///
/// # Errors
/// Returns a database error when the count is negative or past `u32::MAX`.
pub(crate) fn u32_from_count(value: i64, col: &str) -> AppResult<u32> {
    u32::try_from(value)
        .map_err(|e| AppError::database(format!("usage count {col} = {value}: {e}")))
}

/// A byte or millisecond total the engine returns as `i64`, as the `u64`
/// the model carries.
///
/// # Errors
/// Returns a database error when the total is negative.
pub(crate) fn u64_from_sum(value: i64, col: &str) -> AppResult<u64> {
    u64::try_from(value).map_err(|e| AppError::database(format!("usage sum {col} = {value}: {e}")))
}

/// A duration or size the model carries as `u32`, as the `INTEGER` column
/// holds it; a value past `i32::MAX` saturates rather than refusing to
/// record the call.
#[must_use]
pub fn saturating_i32(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// An HTTP status as the `SMALLINT` `api_key_usage.status_code` declares.
///
/// # Errors
/// Returns an invalid-input error when the value is not an HTTP status.
pub fn api_key_status_column(status: u16) -> AppResult<i16> {
    i16::try_from(status).map_err(|e| {
        AppError::invalid_input(format!("status_code {status} is not an HTTP status: {e}"))
    })
}

/// The share of `success` in `total`, in `0.0..=1.0`; an empty total is 0.
#[must_use]
pub fn success_ratio(success: i64, total: i64) -> f64 {
    if total <= 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    {
        success.max(0) as f64 / total as f64
    }
}

/// The first instant of the current UTC month.
///
/// The window the tier's monthly JWT limit is counted over, spelled once
/// for both engines. This used to be a rolling hour on `SQLite`, which
/// made the monthly limit decorative there, and the session time zone's
/// month on Postgres.
#[must_use]
pub fn current_utc_month_start() -> DateTime<Utc> {
    let now = Utc::now();
    now.date_naive()
        .with_day(1)
        .and_then(|first| first.and_hms_opt(0, 0, 0))
        .map_or(now, |midnight| midnight.and_utc())
}

/// A request-log row projected by [`request_logs_sql`].
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn request_log_from_row<R>(row: &R) -> AppResult<RequestLog>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i32: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<i32>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let text = |col: &str| -> AppResult<String> {
        row.try_get(col).map_err(|e| usage_column_error(col, e))
    };
    let opt_int = |col: &str| -> AppResult<Option<i32>> {
        row.try_get(col).map_err(|e| usage_column_error(col, e))
    };
    Ok(RequestLog {
        id: text("id")?,
        timestamp: row
            .try_get("timestamp")
            .map_err(|e| usage_column_error("timestamp", e))?,
        api_key_id: text("api_key_id")?,
        api_key_name: text("api_key_name")?,
        tool_name: text("tool_name")?,
        status_code: row
            .try_get("status_code")
            .map_err(|e| usage_column_error("status_code", e))?,
        response_time_ms: opt_int("response_time_ms")?,
        error_message: row
            .try_get("error_message")
            .map_err(|e| usage_column_error("error_message", e))?,
        request_size_bytes: opt_int("request_size_bytes")?,
        response_size_bytes: opt_int("response_size_bytes")?,
    })
}

/// Emit the whole [`UsageRepository`](super::UsageRepository)
/// implementation for one backend type.
///
/// The body is written once here; each backend's shell invokes it with its
/// own type, its uuid codec from [`super::uuid_columns`], and the two
/// spellings only it knows: the expression that mints a usage row's id
/// (`usage_id`) and the cast an IP address needs (`inet`). sqlx resolves
/// the driver from `self.pool()` per expansion.
///
/// The body names its consts, helpers and types unqualified, so the
/// invoking shell must `use` every one of them.
macro_rules! impl_usage_repository {
    ($ty:ty, $ids:ident, usage_id = $usage_id:literal, inet = $inet:literal) => {
        #[async_trait::async_trait]
        impl UsageRepository for $ty {
            async fn record_api_key(&self, usage: &ApiKeyUsage) -> AppResult<()> {
                sqlx::query(record_api_key_usage_sql!($usage_id, $inet))
                    .bind(&usage.api_key_id)
                    .bind(usage.timestamp)
                    .bind(&usage.tool_name)
                    .bind(api_key_status_column(usage.status_code)?)
                    .bind(usage.response_time_ms.map(saturating_i32))
                    .bind(usage.request_size_bytes.map(saturating_i32))
                    .bind(usage.response_size_bytes.map(saturating_i32))
                    .bind(&usage.ip_address)
                    .bind(&usage.user_agent)
                    .bind(&usage.error_message)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record API key usage: {e}"))
                    })?;
                Ok(())
            }

            async fn get_api_key_current(&self, api_key_id: &str) -> AppResult<u32> {
                // The key's own sliding rate-limit window; a system-level
                // lookup with no user scoping, as the rate limiter has only
                // the key in hand.
                let api_key = ApiKeyRepository::get_by_id(self, api_key_id, None)
                    .await?
                    .ok_or_else(|| AppError::not_found("API key"))?;
                let window_start =
                    Utc::now() - Duration::seconds(i64::from(api_key.rate_limit_window_seconds));
                let row = sqlx::query(API_KEY_CURRENT_USAGE_SQL)
                    .bind(api_key_id)
                    .bind(window_start)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get API key current usage: {e}"))
                    })?;
                let count: i64 = row
                    .try_get("count")
                    .map_err(|e| usage_column_error("count", e))?;
                u32_from_count(count, "count")
            }

            async fn get_api_key_stats(
                &self,
                api_key_id: &str,
                start_date: DateTime<Utc>,
                end_date: DateTime<Utc>,
            ) -> AppResult<ApiKeyUsageStats> {
                let totals = sqlx::query(API_KEY_STATS_SQL)
                    .bind(i32::from(SUCCESS_MIN))
                    .bind(i32::from(SUCCESS_MAX))
                    .bind(i32::from(BAD_REQUEST))
                    .bind(api_key_id)
                    .bind(start_date)
                    .bind(end_date)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get API key usage stats: {e}"))
                    })?;
                let count = |col: &str| -> AppResult<u32> {
                    let value: i64 = totals
                        .try_get(col)
                        .map_err(|e| usage_column_error(col, e))?;
                    u32_from_count(value, col)
                };
                let total_response_time: Option<i64> = totals
                    .try_get("total_response_time")
                    .map_err(|e| usage_column_error("total_response_time", e))?;

                let per_tool = sqlx::query(API_KEY_TOOL_USAGE_SQL)
                    .bind(i32::from(SUCCESS_MIN))
                    .bind(i32::from(SUCCESS_MAX))
                    .bind(api_key_id)
                    .bind(start_date)
                    .bind(end_date)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get API key tool usage stats: {e}"))
                    })?;
                let mut tool_usage = serde_json::Map::new();
                for row in &per_tool {
                    let tool_name: String = row
                        .try_get("endpoint")
                        .map_err(|e| usage_column_error("endpoint", e))?;
                    let tool_count: i64 = row
                        .try_get("tool_count")
                        .map_err(|e| usage_column_error("tool_count", e))?;
                    let success_count: i64 = row
                        .try_get("success_count")
                        .map_err(|e| usage_column_error("success_count", e))?;
                    let avg_response_time: Option<f64> = row
                        .try_get("avg_response_time")
                        .map_err(|e| usage_column_error("avg_response_time", e))?;
                    tool_usage.insert(
                        tool_name,
                        serde_json::json!({
                            "count": tool_count,
                            "success_count": success_count,
                            "avg_response_time_ms": avg_response_time.unwrap_or(0.0),
                            "success_rate": success_ratio(success_count, tool_count),
                        }),
                    );
                }

                Ok(ApiKeyUsageStats {
                    api_key_id: api_key_id.to_owned(),
                    period_start: start_date,
                    period_end: end_date,
                    total_requests: count("total_requests")?,
                    successful_requests: count("successful_requests")?,
                    failed_requests: count("failed_requests")?,
                    total_response_time_ms: total_response_time
                        .map_or(Ok(0), |t| u64_from_sum(t, "total_response_time"))?,
                    tool_usage: serde_json::Value::Object(tool_usage),
                })
            }

            async fn record_jwt_usage(&self, usage: &JwtUsage) -> AppResult<()> {
                sqlx::query(record_jwt_usage_sql!($usage_id, $inet))
                    .bind($ids::bind(usage.user_id))
                    .bind(usage.timestamp)
                    .bind(&usage.endpoint)
                    .bind(&usage.method)
                    .bind(i32::from(usage.status_code))
                    .bind(usage.response_time_ms.map(saturating_i32))
                    .bind(usage.request_size_bytes.map(saturating_i32))
                    .bind(usage.response_size_bytes.map(saturating_i32))
                    .bind(&usage.ip_address)
                    .bind(&usage.user_agent)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to record JWT usage: {e}")))?;
                Ok(())
            }

            async fn get_jwt_current_usage(&self, user_id: Uuid) -> AppResult<u32> {
                let row = sqlx::query(JWT_CURRENT_USAGE_SQL)
                    .bind($ids::bind(user_id))
                    .bind(current_utc_month_start())
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get JWT current usage: {e}"))
                    })?;
                let count: i64 = row
                    .try_get("count")
                    .map_err(|e| usage_column_error("count", e))?;
                u32_from_count(count, "count")
            }

            async fn get_request_logs(
                &self,
                user_id: Option<Uuid>,
                api_key_id: Option<&str>,
                start_time: Option<DateTime<Utc>>,
                end_time: Option<DateTime<Utc>>,
                status_filter: Option<&str>,
                tool_filter: Option<&str>,
            ) -> AppResult<Vec<RequestLog>> {
                let sql = request_logs_sql(RequestLogFilters {
                    user: user_id.is_some(),
                    api_key: api_key_id.is_some(),
                    start: start_time.is_some(),
                    end: end_time.is_some(),
                    status: status_filter.is_some(),
                    tool: tool_filter.is_some(),
                })?;
                let mut query = sqlx::query(&sql);
                if let Some(uid) = user_id {
                    query = query.bind($ids::bind(uid));
                }
                if let Some(key_id) = api_key_id {
                    query = query.bind(key_id.to_owned());
                }
                if let Some(start) = start_time {
                    query = query.bind(start);
                }
                if let Some(end) = end_time {
                    query = query.bind(end);
                }
                if let Some(status) = status_filter {
                    query = query.bind(format!("{status}%"));
                }
                if let Some(tool) = tool_filter {
                    query = query.bind(format!("%{tool}%"));
                }
                query
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get request logs: {e}")))?
                    .iter()
                    .map(request_log_from_row)
                    .collect()
            }

            async fn get_top_tools_analysis(
                &self,
                user_id: Uuid,
                start_time: DateTime<Utc>,
                end_time: DateTime<Utc>,
            ) -> AppResult<Vec<ToolUsage>> {
                let rows = sqlx::query(TOP_TOOLS_SQL)
                    .bind($ids::bind(user_id))
                    .bind(start_time)
                    .bind(end_time)
                    .bind(i32::from(BAD_REQUEST))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get top tools analysis: {e}"))
                    })?;

                let mut tool_usage = Vec::with_capacity(rows.len());
                for row in &rows {
                    let endpoint: String = row
                        .try_get("endpoint")
                        .map_err(|e| usage_column_error("endpoint", e))?;
                    let usage_count: i64 = row
                        .try_get("usage_count")
                        .map_err(|e| usage_column_error("usage_count", e))?;
                    let success_count: i64 = row
                        .try_get("success_count")
                        .map_err(|e| usage_column_error("success_count", e))?;
                    let error_count: i64 = row
                        .try_get("error_count")
                        .map_err(|e| usage_column_error("error_count", e))?;
                    let avg_response_time: Option<f64> = row
                        .try_get("avg_response_time")
                        .map_err(|e| usage_column_error("avg_response_time", e))?;

                    let error_rate = success_ratio(error_count, usage_count);
                    if error_rate > 0.1 {
                        warn!(
                            endpoint = %endpoint,
                            error_count,
                            usage_count,
                            "High error rate for endpoint: {:.2}%",
                            error_rate * 100.0
                        );
                    }

                    tool_usage.push(ToolUsage {
                        tool_name: endpoint,
                        request_count: u64_from_sum(usage_count, "usage_count")?,
                        success_rate: success_ratio(success_count, usage_count) * 100.0,
                        average_response_time: avg_response_time.unwrap_or(0.0),
                    });
                }
                Ok(tool_usage)
            }
        }
    };
}
pub(crate) use impl_usage_repository;
