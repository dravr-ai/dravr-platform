// ABOUTME: Repository trait for the days on which an athlete met a first-run flow's done rule
// ABOUTME: The SQL is written once here and both backends emit their impl from it with their own uuid codec

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! First-run progress (carnet#1099).
//!
//! The four first-run flows are use-case catalogue entries; what makes each
//! one done happens in a turn, a tap or a dossier read, and none of those is
//! kept. A row here records that a flow's done rule was met on one of the
//! athlete's local days. Most flows are done at their first row; the daily
//! check-in counts distinct days, which is why the day is part of the key.
//!
//! Every statement is keyed by both `tenant_id` and `user_id`.

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

/// The format `local_date` is stored in on both engines.
const LOCAL_DATE_FORMAT: &str = "%Y-%m-%d";

/// One day on which an athlete met a first-run flow's done rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstRunDay {
    /// The flow's catalogue id.
    pub flow_id: String,
    /// The athlete's civil date the rule was met on.
    pub local_date: NaiveDate,
    /// When the row was written.
    pub recorded_at: DateTime<Utc>,
}

/// The days each athlete met each first-run flow's done rule.
#[async_trait]
pub trait FirstRunProgressRepository: Send + Sync {
    /// Record that `user_id` met `flow_id`'s done rule on `local_date`, at
    /// `recorded_at`. Returns `true` when this call wrote the row, `false`
    /// when that day was already recorded for the flow.
    async fn record_first_run_day(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        flow_id: &str,
        local_date: NaiveDate,
        recorded_at: DateTime<Utc>,
    ) -> AppResult<bool>;

    /// Every day `user_id` met any first-run flow's done rule, oldest first.
    async fn first_run_days(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
    ) -> AppResult<Vec<FirstRunDay>>;
}

/// The primary key is the day: a second write of the same day changes
/// nothing, and the affected count says whether this call wrote it.
pub(crate) const RECORD_FIRST_RUN_DAY_SQL: &str = r"
            INSERT INTO first_run_progress (tenant_id, user_id, flow_id, local_date, recorded_at)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (tenant_id, user_id, flow_id, local_date) DO NOTHING
            ";

pub(crate) const FIRST_RUN_DAYS_SQL: &str = r"
            SELECT flow_id, local_date, recorded_at
            FROM first_run_progress
            WHERE tenant_id = $1 AND user_id = $2
            ORDER BY recorded_at, flow_id
            ";

/// The row tuple [`FIRST_RUN_DAYS_SQL`] selects.
pub(crate) type FirstRunDayRow = (String, String, i64);

/// `local_date` as the text both engines store it as.
pub(crate) fn local_date_text(date: NaiveDate) -> String {
    date.format(LOCAL_DATE_FORMAT).to_string()
}

/// A stored row as the day it records.
///
/// # Errors
/// Returns a database error for a date or an instant that does not parse,
/// which only a hand-edited row can hold.
pub(crate) fn first_run_day_from_row(
    (flow_id, local_date, recorded_at): FirstRunDayRow,
) -> AppResult<FirstRunDay> {
    let local_date = NaiveDate::parse_from_str(&local_date, LOCAL_DATE_FORMAT).map_err(|e| {
        AppError::database(format!(
            "first_run_progress.local_date {local_date:?} for {flow_id} is not a date: {e}"
        ))
    })?;
    let recorded_at = DateTime::from_timestamp_millis(recorded_at).ok_or_else(|| {
        AppError::database(format!(
            "first_run_progress.recorded_at {recorded_at} is not a representable instant"
        ))
    })?;
    Ok(FirstRunDay {
        flow_id,
        local_date,
        recorded_at,
    })
}

/// Emit the whole [`FirstRunProgressRepository`] implementation for one
/// backend type. `$ids` is that backend's uuid codec
/// ([`crate::repositories::uuid_columns`]): `TEXT` ids on `SQLite`, native
/// `uuid` on Postgres.
macro_rules! impl_first_run_progress_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl FirstRunProgressRepository for $ty {
            async fn record_first_run_day(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                flow_id: &str,
                local_date: NaiveDate,
                recorded_at: DateTime<Utc>,
            ) -> AppResult<bool> {
                let written = sqlx::query(RECORD_FIRST_RUN_DAY_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .bind(flow_id)
                    .bind(local_date_text(local_date))
                    .bind(recorded_at.timestamp_millis())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record a first-run day: {e}"))
                    })?
                    .rows_affected();
                Ok(written > 0)
            }

            async fn first_run_days(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
            ) -> AppResult<Vec<FirstRunDay>> {
                let rows: Vec<FirstRunDayRow> = sqlx::query_as(FIRST_RUN_DAYS_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read first-run progress: {e}"))
                    })?;
                rows.into_iter().map(first_run_day_from_row).collect()
            }
        }
    };
}
pub(crate) use impl_first_run_progress_repository;
