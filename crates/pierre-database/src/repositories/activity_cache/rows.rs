// ABOUTME: Decoding activity cache rows of either backend: the activity with its detail, coverage, failures, freshness
// ABOUTME: Generic over sqlx rows so one decoder serves SQLite and Postgres; each names the column it could not read

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::Activity;
use serde_json::{Map, Value};

use super::super::activity_route_tracks::joined_route_outcome;
use super::{
    ActivityFetchFailure, ActivityFetchFailureRecord, BackfillCoverage, CachedActivityRow,
    CaptureFreshness,
};

/// Later of the row-derived sync signal and the fetch-freshness mark.
pub(crate) fn later(a: Option<DateTime<Utc>>, b: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

/// Read one timestamp column of either backend, via `try_get` only — a
/// corrupt row surfaces as a recoverable error naming the column, never as a
/// panic. On `SQLite` the column is RFC3339 text and sqlx parses it; on
/// Postgres it is a TIMESTAMPTZ.
///
/// # Errors
/// Returns a database error naming the column when it cannot be decoded.
pub(crate) fn timestamp_column<R>(row: &R, column: &str) -> AppResult<DateTime<Utc>>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    row.try_get(column)
        .map_err(|e| AppError::database(format!("activity col {column}: {e}")))
}

/// The failure record one `activity_fetch_failures` row holds.
///
/// # Errors
/// Returns a database error when a column is missing or does not decode.
pub(crate) fn fetch_failure_from_row<R>(row: &R) -> AppResult<ActivityFetchFailureRecord>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i32: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let count = |column: &str| -> AppResult<u32> {
        let value: i32 = row
            .try_get(column)
            .map_err(|e| AppError::database(format!("fetch failure col {column}: {e}")))?;
        Ok(u32::try_from(value).unwrap_or(1))
    };
    let reason: String = row
        .try_get("reason")
        .map_err(|e| AppError::database(format!("fetch failure col reason: {e}")))?;
    let failure = ActivityFetchFailure::from_slug(&reason)
        .ok_or_else(|| AppError::database(format!("fetch failure reason {reason:?} is unknown")))?;
    Ok(ActivityFetchFailureRecord {
        failed_at: timestamp_column(row, "failed_at")?,
        failure,
        consecutive: count("consecutive")?,
        streak: count("streak")?,
        streak_started_at: timestamp_column(row, "streak_started_at")?,
    })
}

/// Deserialize the stored `Activity` out of one row's `data_json`, with the
/// fields its list copy lacks filled from the row's `detail_json`.
///
/// # Errors
/// Returns a database error when a column is missing, `data_json` does not
/// hold an `Activity`, or `detail_json` is not a JSON object.
pub(crate) fn activity_from_row<R>(row: &R) -> AppResult<Activity>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let data_json: String = row
        .try_get("data_json")
        .map_err(|e| AppError::database(format!("activity col data_json: {e}")))?;
    let detail_json: Option<String> = row
        .try_get("detail_json")
        .map_err(|e| AppError::database(format!("activity col detail_json: {e}")))?;
    let Some(detail_json) = detail_json else {
        return serde_json::from_str::<Activity>(&data_json).map_err(|e| {
            AppError::database(format!("Failed to deserialize cached activity: {e}"))
        });
    };
    let mut activity = json_object(&data_json, "data_json")?;
    fill_missing(&mut activity, json_object(&detail_json, "detail_json")?);
    serde_json::from_value::<Activity>(Value::Object(activity))
        .map_err(|e| AppError::database(format!("Failed to deserialize cached activity: {e}")))
}

/// One stored JSON column parsed as an object.
fn json_object(json: &str, column: &str) -> AppResult<Map<String, Value>> {
    match serde_json::from_str::<Value>(json) {
        Ok(Value::Object(object)) => Ok(object),
        Ok(_) => Err(AppError::database(format!(
            "activity col {column} is not a JSON object"
        ))),
        Err(e) => Err(AppError::database(format!(
            "activity col {column} does not parse: {e}"
        ))),
    }
}

/// Give `activity` every field of `detail` it lacks. A field the list copy
/// already carries is its own read of the activity, and stands.
fn fill_missing(activity: &mut Map<String, Value>, detail: Map<String, Value>) {
    for (key, value) in detail {
        let slot = activity.entry(key).or_insert(Value::Null);
        if slot.is_null() {
            *slot = value;
        }
    }
}

/// Read a [`CachedActivityRow`] out of one row of
/// [`GET_CACHED_ACTIVITY_ROWS_SQL`].
///
/// # Errors
/// Returns a database error when a column is missing or `data_json` does not
/// hold an `Activity`.
pub(crate) fn cached_activity_row_from_row<R>(row: &R) -> AppResult<CachedActivityRow>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let provider: String = row
        .try_get("provider")
        .map_err(|e| AppError::database(format!("activity col provider: {e}")))?;
    let detail_read: bool = row
        .try_get("detail_settled")
        .map_err(|e| AppError::database(format!("activity col detail_settled: {e}")))?;
    Ok(CachedActivityRow {
        provider,
        activity: activity_from_row(row)?,
        route: joined_route_outcome(row)?,
        detail_read,
    })
}

/// Extract a [`BackfillCoverage`] from a row of either backend.
/// `hit_feed_end` decodes as `bool` on both: a BOOLEAN on Postgres, an
/// INTEGER 0/1 on `SQLite`.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn backfill_coverage_from_row<R>(row: &R) -> AppResult<BackfillCoverage>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let capture_version: i64 = row
        .try_get("capture_version")
        .map_err(|e| AppError::database(format!("coverage col capture_version: {e}")))?;
    Ok(BackfillCoverage {
        oldest_reached_ts: row
            .try_get("oldest_reached_ts")
            .map_err(|e| AppError::database(format!("coverage col oldest_reached_ts: {e}")))?,
        hit_feed_end: row
            .try_get("hit_feed_end")
            .map_err(|e| AppError::database(format!("coverage col hit_feed_end: {e}")))?,
        // BIGINT on Postgres and INTEGER on `SQLite`, both of which decode as
        // i64. Written from a u32 and constrained NOT NULL, so a value outside
        // that range can only be a hand-edited row; reading it as the baseline
        // re-captures rather than trusts it.
        capture_version: u32::try_from(capture_version).unwrap_or(0),
    })
}

/// The `provider` column of a row of either backend.
///
/// # Errors
/// Returns a database error when the column cannot be decoded.
pub(crate) fn provider_column<R>(row: &R) -> AppResult<String>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    row.try_get("provider")
        .map_err(|e| AppError::database(format!("cached activity col provider: {e}")))
}

/// Extract a [`CaptureFreshness`] from a row of either backend. The two
/// nullable timestamps decode as `Option<DateTime<Utc>>`: a NULL is `None`
/// on both engines, and a value is RFC3339 text on `SQLite` — the only form
/// `provider_connections.last_used_at` is ever written in — or a
/// TIMESTAMPTZ on Postgres.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded.
pub(crate) fn capture_freshness_from_row<R>(row: &R) -> AppResult<CaptureFreshness>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let col = |name: &str| -> AppResult<String> {
        row.try_get(name)
            .map_err(|e| AppError::database(format!("capture col {name}: {e}")))
    };
    let stamp = |name: &str| -> AppResult<Option<DateTime<Utc>>> {
        row.try_get(name)
            .map_err(|e| AppError::database(format!("capture col {name}: {e}")))
    };
    Ok(CaptureFreshness {
        tenant_id: col("tenant_id")?,
        user_id: col("user_id")?,
        provider: col("provider")?,
        last_used_at: stamp("last_used_at")?,
        last_fetch_at: stamp("fetched_at")?,
    })
}
