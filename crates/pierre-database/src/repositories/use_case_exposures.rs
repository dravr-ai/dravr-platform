// ABOUTME: Repository trait for how often each use-case starter was shown to an athlete, and when they last tapped it
// ABOUTME: The SQL is written once here and both backends emit their impl from it with their own uuid codec

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Use-case starter exposures (carnet#828).
//!
//! The starter ranker reads these to stop offering what the athlete has
//! already used or keeps ignoring: a `once` starter goes after its first tap,
//! and any starter shown three times since its last tap goes too. A welcome
//! that offers a starter records it shown; a tap resets the count.
//!
//! Every statement is keyed by both `tenant_id` and `user_id`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

/// One starter's exposure to one athlete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseCaseExposure {
    /// The catalogue id.
    pub use_case_id: String,
    /// Welcomes that offered it since its last tap.
    pub shown_count: u32,
    /// When a welcome last offered it.
    pub last_shown_at: Option<DateTime<Utc>>,
    /// When the athlete last tapped it.
    pub tapped_at: Option<DateTime<Utc>>,
}

/// The starters each athlete was shown and tapped.
#[async_trait]
pub trait UseCaseExposureRepository: Send + Sync {
    /// Count each of `use_case_ids` shown to `user_id` at `shown_at`.
    async fn record_use_cases_shown(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        use_case_ids: &[&str],
        shown_at: DateTime<Utc>,
    ) -> AppResult<()>;

    /// Record that `user_id` tapped `use_case_id` at `tapped_at`, which
    /// restarts its shown count.
    async fn record_use_case_tapped(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        use_case_id: &str,
        tapped_at: DateTime<Utc>,
    ) -> AppResult<()>;

    /// Every starter `user_id` was shown or tapped, by id.
    async fn use_case_exposures(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
    ) -> AppResult<Vec<UseCaseExposure>>;
}

/// `excluded` names the row the insert proposed on both engines.
pub(crate) const RECORD_USE_CASE_SHOWN_SQL: &str = r"
            INSERT INTO use_case_exposures
                (tenant_id, user_id, use_case_id, shown_count, last_shown_at_ms, tapped_at_ms)
            VALUES ($1, $2, $3, 1, $4, NULL)
            ON CONFLICT (tenant_id, user_id, use_case_id) DO UPDATE SET
                shown_count = use_case_exposures.shown_count + 1,
                last_shown_at_ms = excluded.last_shown_at_ms
            ";

pub(crate) const RECORD_USE_CASE_TAPPED_SQL: &str = r"
            INSERT INTO use_case_exposures
                (tenant_id, user_id, use_case_id, shown_count, last_shown_at_ms, tapped_at_ms)
            VALUES ($1, $2, $3, 0, NULL, $4)
            ON CONFLICT (tenant_id, user_id, use_case_id) DO UPDATE SET
                shown_count = 0,
                tapped_at_ms = excluded.tapped_at_ms
            ";

pub(crate) const USE_CASE_EXPOSURES_SQL: &str = r"
            SELECT use_case_id, shown_count, last_shown_at_ms, tapped_at_ms
            FROM use_case_exposures
            WHERE tenant_id = $1 AND user_id = $2
            ORDER BY use_case_id
            ";

/// The row tuple [`USE_CASE_EXPOSURES_SQL`] selects.
pub(crate) type ExposureRow = (String, i64, Option<i64>, Option<i64>);

/// A stored row as the exposure it records.
///
/// # Errors
/// Returns a database error for a negative count or an instant outside
/// chrono's range, which only a hand-edited row can hold.
pub(crate) fn exposure_from_row(
    (use_case_id, shown_count, last_shown_at_ms, tapped_at_ms): ExposureRow,
) -> AppResult<UseCaseExposure> {
    let instant = |ms: Option<i64>, column: &str| {
        ms.map(|ms| {
            DateTime::from_timestamp_millis(ms).ok_or_else(|| {
                AppError::database(format!(
                    "use_case_exposures.{column} {ms} is not a representable instant"
                ))
            })
        })
        .transpose()
    };
    Ok(UseCaseExposure {
        shown_count: u32::try_from(shown_count).map_err(|_| {
            AppError::database(format!(
                "use_case_exposures.shown_count {shown_count} for {use_case_id} is out of range"
            ))
        })?,
        last_shown_at: instant(last_shown_at_ms, "last_shown_at_ms")?,
        tapped_at: instant(tapped_at_ms, "tapped_at_ms")?,
        use_case_id,
    })
}

/// Emit the whole [`UseCaseExposureRepository`] implementation for one
/// backend type. `$ids` is that backend's uuid codec
/// ([`crate::repositories::uuid_columns`]): `TEXT` ids on `SQLite`, native
/// `uuid` on Postgres.
macro_rules! impl_use_case_exposure_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl UseCaseExposureRepository for $ty {
            async fn record_use_cases_shown(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                use_case_ids: &[&str],
                shown_at: DateTime<Utc>,
            ) -> AppResult<()> {
                for use_case_id in use_case_ids {
                    sqlx::query(RECORD_USE_CASE_SHOWN_SQL)
                        .bind($ids::bind(tenant_id.as_uuid()))
                        .bind($ids::bind(user_id))
                        .bind(*use_case_id)
                        .bind(shown_at.timestamp_millis())
                        .execute(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to record a use case shown: {e}"))
                        })?;
                }
                Ok(())
            }

            async fn record_use_case_tapped(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                use_case_id: &str,
                tapped_at: DateTime<Utc>,
            ) -> AppResult<()> {
                sqlx::query(RECORD_USE_CASE_TAPPED_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .bind(use_case_id)
                    .bind(tapped_at.timestamp_millis())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record a use case tapped: {e}"))
                    })?;
                Ok(())
            }

            async fn use_case_exposures(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
            ) -> AppResult<Vec<UseCaseExposure>> {
                let rows: Vec<ExposureRow> = sqlx::query_as(USE_CASE_EXPOSURES_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read use case exposures: {e}"))
                    })?;
                rows.into_iter().map(exposure_from_row).collect()
            }
        }
    };
}
pub(crate) use impl_use_case_exposure_repository;
