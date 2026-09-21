// ABOUTME: Shared statements, row decode and body for the usage_counters table both backends serve
// ABOUTME: One SQL text per operation; the upsert returns the row it produced so a caller sees its own increment
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Usage counters, written once.
//!
//! A `usage_counters` row is one `(tenant_id, user_id, counter_key, period)`
//! bucket holding an integer `value`. Every key column is `TEXT` on both
//! backends and binds as the text the caller holds; `period` is a
//! caller-formatted bucket label (a day or a month), so the pruning compare
//! is lexical on both engines by design. `updated_at` is `TIMESTAMPTZ` on
//! Postgres and RFC 3339 text on `SQLite`; it binds as [`DateTime<Utc>`] on
//! both (sqlx-sqlite writes the bytes `to_rfc3339()` produces) and is read
//! back as one, rendered RFC 3339 for the record. Nothing differs per
//! engine, so the macro takes only the backend type.

use std::fmt::Display;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::UsageCounterRecord;
use sqlx::Row;

/// The columns [`counter_from_row`] reads, in the order every statement
/// projects them.
macro_rules! counter_columns {
    () => {
        "tenant_id, user_id, counter_key, period, value, updated_at"
    };
}

/// Add `$5` to a bucket, creating it at `$5` when absent, and return the row
/// the statement produced.
///
/// `RETURNING`, not a follow-up `SELECT`: the upsert and the read must be one
/// statement or the value a caller sees is not the value its own increment
/// produced. Two concurrent turns can both land their increments before
/// either reads, and both then see 2; a caller using `value == 1` to claim a
/// once-per-window slot — the quota notice does exactly that — would have
/// both turns decline it, and the athlete is never told about their budget
/// (registre#258).
pub(crate) const INCREMENT_COUNTER_SQL: &str = concat!(
    "INSERT INTO usage_counters (tenant_id, user_id, counter_key, period, value, updated_at) \
     VALUES ($1, $2, $3, $4, $5, $6) \
     ON CONFLICT (tenant_id, user_id, counter_key, period) \
     DO UPDATE SET value = usage_counters.value + EXCLUDED.value, updated_at = EXCLUDED.updated_at \
     RETURNING ",
    counter_columns!()
);

/// One bucket, or no row when it was never incremented.
pub(crate) const GET_COUNTER_SQL: &str = concat!(
    "SELECT ",
    counter_columns!(),
    " FROM usage_counters \
      WHERE tenant_id = $1 AND user_id = $2 AND counter_key = $3 AND period = $4"
);

/// Every bucket whose period label sorts before `$1`, across all tenants:
/// system housekeeping reached only from the background pruning task.
pub(crate) const DELETE_OLD_COUNTERS_SQL: &str = "DELETE FROM usage_counters WHERE period < $1";

/// The error a column that will not decode surfaces as, named after the
/// column so a corrupt row is locatable from the message.
pub(crate) fn counter_column_error(col: &str, e: impl Display) -> AppError {
    AppError::database(format!("usage_counters column {col}: {e}"))
}

/// A counter row projected by [`counter_columns!`].
///
/// # Errors
/// Returns a database error when a column is missing or will not decode.
pub(crate) fn counter_from_row<R>(row: &R) -> AppResult<UsageCounterRecord>
where
    R: Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    for<'a> String: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
    for<'a> i64: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
    for<'a> DateTime<Utc>: sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
{
    let text = |col: &str| -> AppResult<String> {
        row.try_get(col).map_err(|e| counter_column_error(col, e))
    };
    let updated_at: DateTime<Utc> = row
        .try_get("updated_at")
        .map_err(|e| counter_column_error("updated_at", e))?;
    Ok(UsageCounterRecord {
        tenant_id: text("tenant_id")?,
        user_id: text("user_id")?,
        counter_key: text("counter_key")?,
        period: text("period")?,
        value: row
            .try_get("value")
            .map_err(|e| counter_column_error("value", e))?,
        updated_at: updated_at.to_rfc3339(),
    })
}

/// Emit the whole [`UsageCounterRepository`](super::UsageCounterRepository)
/// implementation for one backend type.
///
/// The body is written once here; each backend's shell invokes it with its
/// own type. Nothing in these statements differs per engine. The body names
/// its consts and helpers unqualified, so the invoking shell must `use`
/// every one of them.
macro_rules! impl_usage_counter_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl UsageCounterRepository for $ty {
            async fn increment_counter(
                &self,
                tenant_id: &str,
                user_id: &str,
                counter_key: &str,
                period: &str,
                amount: i64,
            ) -> AppResult<UsageCounterRecord> {
                let row = sqlx::query(INCREMENT_COUNTER_SQL)
                    .bind(tenant_id)
                    .bind(user_id)
                    .bind(counter_key)
                    .bind(period)
                    .bind(amount)
                    .bind(Utc::now())
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to increment usage counter: {e}"))
                    })?;
                counter_from_row(&row)
            }

            async fn get_counter(
                &self,
                tenant_id: &str,
                user_id: &str,
                counter_key: &str,
                period: &str,
            ) -> AppResult<UsageCounterRecord> {
                let row = sqlx::query(GET_COUNTER_SQL)
                    .bind(tenant_id)
                    .bind(user_id)
                    .bind(counter_key)
                    .bind(period)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get usage counter: {e}")))?;
                match row {
                    Some(row) => counter_from_row(&row),
                    None => Ok(UsageCounterRecord {
                        tenant_id: tenant_id.to_owned(),
                        user_id: user_id.to_owned(),
                        counter_key: counter_key.to_owned(),
                        period: period.to_owned(),
                        value: 0,
                        updated_at: String::new(),
                    }),
                }
            }

            async fn delete_old_counters(&self, period_before: &str) -> AppResult<u64> {
                let result = sqlx::query(DELETE_OLD_COUNTERS_SQL)
                    .bind(period_before)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete old usage counters: {e}"))
                    })?;
                Ok(result.rows_affected())
            }
        }
    };
}
pub(crate) use impl_usage_counter_repository;
