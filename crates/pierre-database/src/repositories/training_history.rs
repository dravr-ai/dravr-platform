// ABOUTME: Shared statements and body for the daily training-state history behind /api/v1/endurance/history
// ABOUTME: One SQL text per operation; each backend shell supplies only how it binds a user id

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Training history, written once.
//!
//! One row per `(tenant_id, user_id, date)` holding the day's computed load
//! state; the upsert replaces every metric on conflict so a recompute is
//! idempotent.
//!
//! The two backends differ in one respect only: `user_id` is a `uuid` column
//! on Postgres and `TEXT` on `SQLite`, so the id is bound natively on one and
//! stringified on the other. That conversion is the macro's single argument.
//! `tenant_id` binds as a `TenantId`, whose own sqlx encoding is already
//! text on `SQLite` and a native uuid on Postgres; `date` binds and decodes as
//! a [`NaiveDate`], which sqlx writes as ISO `YYYY-MM-DD` text on `SQLite` and
//! a native `DATE` on Postgres, so `BETWEEN` and `ORDER BY` agree on both.
//!
//! `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
//! Postgres, so one statement serves both drivers and cannot drift between
//! them. `CURRENT_TIMESTAMP` is the spelling both engines accept.

use chrono::NaiveDate;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::DailyTrainingState;

/// Insert or replace one day's state.
pub(crate) const UPSERT_TRAINING_HISTORY_SQL: &str = r"
            INSERT INTO training_history (
                tenant_id, user_id, date, ctl, atl, tsb,
                acwr, monotony, strain, ramp_rate, daily_load, computed_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, CURRENT_TIMESTAMP)
            ON CONFLICT (tenant_id, user_id, date) DO UPDATE SET
                ctl = EXCLUDED.ctl,
                atl = EXCLUDED.atl,
                tsb = EXCLUDED.tsb,
                acwr = EXCLUDED.acwr,
                monotony = EXCLUDED.monotony,
                strain = EXCLUDED.strain,
                ramp_rate = EXCLUDED.ramp_rate,
                daily_load = EXCLUDED.daily_load,
                computed_at = CURRENT_TIMESTAMP
            ";

/// The columns every read decodes, in the order [`training_state_from_row`]
/// reads them — one list for the range and latest queries, so a metric added
/// to [`DailyTrainingState`] reaches both at once.
macro_rules! training_history_columns {
    () => {
        "date, ctl, atl, tsb, acwr, monotony, strain, ramp_rate, daily_load"
    };
}

/// Every day inside an inclusive date window, oldest first.
pub(crate) const GET_TRAINING_HISTORY_SQL: &str = concat!(
    "
            SELECT ",
    training_history_columns!(),
    "
            FROM training_history
            WHERE tenant_id = $1 AND user_id = $2
              AND date BETWEEN $3 AND $4
            ORDER BY date ASC
            "
);

/// Drop every day inside an inclusive date window.
pub(crate) const DELETE_TRAINING_HISTORY_RANGE_SQL: &str = r"
            DELETE FROM training_history
            WHERE tenant_id = $1 AND user_id = $2
              AND date BETWEEN $3 AND $4
            ";

/// The most recent day on record.
pub(crate) const LATEST_TRAINING_HISTORY_SQL: &str = concat!(
    "
            SELECT ",
    training_history_columns!(),
    "
            FROM training_history
            WHERE tenant_id = $1 AND user_id = $2
            ORDER BY date DESC
            LIMIT 1
            "
);

/// Decode one history row. The three `NOT NULL DEFAULT 0.0` metrics fall
/// back to zero on a decode surprise, matching the column default; the
/// nullable derived metrics are decoded strictly so a corrupt value surfaces
/// as an error rather than as `None`.
///
/// # Errors
/// Returns a database error when `date` or a nullable metric cannot be
/// decoded.
pub(crate) fn training_state_from_row<R>(row: &R) -> AppResult<DailyTrainingState>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    NaiveDate: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    f64: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<f64>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let date: NaiveDate = row
        .try_get("date")
        .map_err(|e| AppError::database(format!("read date: {e}")))?;
    let optional = |col: &str| -> AppResult<Option<f64>> {
        row.try_get::<Option<f64>, _>(col)
            .map_err(|e| AppError::database(format!("read {col}: {e}")))
    };
    Ok(DailyTrainingState {
        date,
        ctl: row.try_get("ctl").unwrap_or(0.0),
        atl: row.try_get("atl").unwrap_or(0.0),
        tsb: row.try_get("tsb").unwrap_or(0.0),
        acwr: optional("acwr")?,
        monotony: optional("monotony")?,
        strain: optional("strain")?,
        ramp_rate: optional("ramp_rate")?,
        daily_load: row.try_get("daily_load").unwrap_or(0.0),
    })
}

/// Emit the whole [`TrainingHistoryRepository`] implementation for one
/// backend type.
///
/// `$bind_id` is the function turning a `Uuid` into whatever that backend's
/// `user_id` column accepts: the `bind` of its codec in [`super::uuid_columns`]
/// (`TextUuid` on `SQLite`, `NativeUuid` on Postgres).
///
/// The body is written once here; each backend's shell invokes it with its
/// own type, and sqlx resolves the driver from `self.pool()` per expansion.
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
macro_rules! impl_training_history_repository {
    ($ty:ty, $bind_id:path) => {
        #[async_trait::async_trait]
        impl TrainingHistoryRepository for $ty {
            async fn upsert_training_history_day(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                state: &DailyTrainingState,
            ) -> AppResult<()> {
                sqlx::query(UPSERT_TRAINING_HISTORY_SQL)
                    .bind(tenant_id)
                    .bind($bind_id(user_id))
                    .bind(state.date)
                    .bind(state.ctl)
                    .bind(state.atl)
                    .bind(state.tsb)
                    .bind(state.acwr)
                    .bind(state.monotony)
                    .bind(state.strain)
                    .bind(state.ramp_rate)
                    .bind(state.daily_load)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("upsert_training_history_day: {e}")))?;
                Ok(())
            }

            async fn upsert_training_history_batch(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                states: &[DailyTrainingState],
            ) -> AppResult<()> {
                let mut tx = self
                    .pool()
                    .begin()
                    .await
                    .map_err(|e| AppError::database(format!("begin tx: {e}")))?;
                for state in states {
                    sqlx::query(UPSERT_TRAINING_HISTORY_SQL)
                        .bind(tenant_id)
                        .bind($bind_id(user_id))
                        .bind(state.date)
                        .bind(state.ctl)
                        .bind(state.atl)
                        .bind(state.tsb)
                        .bind(state.acwr)
                        .bind(state.monotony)
                        .bind(state.strain)
                        .bind(state.ramp_rate)
                        .bind(state.daily_load)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| {
                            AppError::database(format!("upsert_training_history_batch: {e}"))
                        })?;
                }
                tx.commit()
                    .await
                    .map_err(|e| AppError::database(format!("commit tx: {e}")))?;
                Ok(())
            }

            async fn get_training_history(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                from: NaiveDate,
                to: NaiveDate,
            ) -> AppResult<Vec<DailyTrainingState>> {
                let rows = sqlx::query(GET_TRAINING_HISTORY_SQL)
                    .bind(tenant_id)
                    .bind($bind_id(user_id))
                    .bind(from)
                    .bind(to)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("get_training_history: {e}")))?;

                rows.iter().map(training_state_from_row).collect()
            }

            async fn delete_training_history_range(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                from: NaiveDate,
                to: NaiveDate,
            ) -> AppResult<u64> {
                let result = sqlx::query(DELETE_TRAINING_HISTORY_RANGE_SQL)
                    .bind(tenant_id)
                    .bind($bind_id(user_id))
                    .bind(from)
                    .bind(to)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("delete_training_history_range: {e}"))
                    })?;

                Ok(result.rows_affected())
            }

            async fn latest_training_history(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
            ) -> AppResult<Option<DailyTrainingState>> {
                let row = sqlx::query(LATEST_TRAINING_HISTORY_SQL)
                    .bind(tenant_id)
                    .bind($bind_id(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("latest_training_history: {e}")))?;

                row.as_ref().map(training_state_from_row).transpose()
            }
        }
    };
}
pub(crate) use impl_training_history_repository;
