// ABOUTME: Repository trait for the warnings the Strava seat-reclaim sweeper sent, one per athlete and tenant
// ABOUTME: The SQL is written once here and both backends emit their impl from it with their own uuid codec

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Warnings sent before a Strava seat is reclaimed.
//!
//! A reclaim disconnects an athlete's Strava grant, so it is only legitimate
//! after the athlete was told and given time to come back. The warning is the
//! evidence of that, which is why it is stored rather than inferred: the
//! sweeper disconnects a holder only when a row here reached the athlete, is
//! at least `warn_lead_days` old, and the athlete has not been active since it.
//!
//! Every statement is keyed by both `user_id` and `tenant_id`: a row is the
//! warning about one Strava token, and an athlete holding one in two tenants
//! is warned about each.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{StravaSeatReclaimWarning, TenantId};
use uuid::Uuid;

/// The warnings the seat-reclaim sweeper has sent.
#[async_trait]
pub trait StravaSeatReclaimWarningRepository: Send + Sync {
    /// The warning `user_id` was sent about the Strava seat their token in
    /// `tenant_id` holds, or `None` when none is stored.
    async fn get_seat_reclaim_warning(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<StravaSeatReclaimWarning>>;

    /// Record that `user_id` was warned at `warned_at`, and whether the
    /// warning reached them outside the app, replacing an earlier warning
    /// about the same token.
    async fn record_seat_reclaim_warning(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        warned_at: DateTime<Utc>,
        reached: bool,
    ) -> AppResult<()>;

    /// Delete the warning about `user_id`'s token in `tenant_id`, returning
    /// whether one existed.
    async fn clear_seat_reclaim_warning(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<bool>;
}

pub(crate) const GET_SEAT_RECLAIM_WARNING_SQL: &str = r"
            SELECT warned_at_ms, reached
            FROM strava_seat_reclaim_warnings
            WHERE user_id = $1 AND tenant_id = $2
            ";

/// `excluded` names the row the insert proposed on both engines.
pub(crate) const RECORD_SEAT_RECLAIM_WARNING_SQL: &str = r"
            INSERT INTO strava_seat_reclaim_warnings (user_id, tenant_id, warned_at_ms, reached)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (user_id, tenant_id) DO UPDATE SET
                warned_at_ms = excluded.warned_at_ms,
                reached = excluded.reached
            ";

pub(crate) const CLEAR_SEAT_RECLAIM_WARNING_SQL: &str = r"
            DELETE FROM strava_seat_reclaim_warnings
            WHERE user_id = $1 AND tenant_id = $2
            ";

/// A stored `warned_at_ms` as the instant it records.
///
/// # Errors
/// Returns a database error for a value outside chrono's representable range,
/// which only a hand-edited row can hold.
pub(crate) fn warned_at_from_ms(ms: i64) -> AppResult<DateTime<Utc>> {
    DateTime::from_timestamp_millis(ms).ok_or_else(|| {
        AppError::database(format!(
            "strava_seat_reclaim_warnings.warned_at_ms {ms} is not a representable instant"
        ))
    })
}

/// Emit the whole [`StravaSeatReclaimWarningRepository`] implementation for
/// one backend type. `$ids` is that backend's uuid codec
/// ([`crate::repositories::uuid_columns`]): `TEXT` ids on `SQLite`, native
/// `uuid` on Postgres.
macro_rules! impl_strava_seat_reclaim_warning_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl StravaSeatReclaimWarningRepository for $ty {
            async fn get_seat_reclaim_warning(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<Option<StravaSeatReclaimWarning>> {
                let row: Option<(i64, bool)> = sqlx::query_as(GET_SEAT_RECLAIM_WARNING_SQL)
                    .bind($ids::bind(user_id))
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read seat reclaim warning: {e}"))
                    })?;
                row.map(|(ms, reached)| {
                    Ok(StravaSeatReclaimWarning {
                        warned_at: warned_at_from_ms(ms)?,
                        reached,
                    })
                })
                .transpose()
            }

            async fn record_seat_reclaim_warning(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                warned_at: DateTime<Utc>,
                reached: bool,
            ) -> AppResult<()> {
                sqlx::query(RECORD_SEAT_RECLAIM_WARNING_SQL)
                    .bind($ids::bind(user_id))
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind(warned_at.timestamp_millis())
                    .bind(reached)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record seat reclaim warning: {e}"))
                    })?;
                Ok(())
            }

            async fn clear_seat_reclaim_warning(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let result = sqlx::query(CLEAR_SEAT_RECLAIM_WARNING_SQL)
                    .bind($ids::bind(user_id))
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to clear seat reclaim warning: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }
        }
    };
}
pub(crate) use impl_strava_seat_reclaim_warning_repository;
