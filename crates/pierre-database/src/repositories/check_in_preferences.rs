// ABOUTME: Repository trait for how often the coach checks in with each athlete, as the athlete chose it
// ABOUTME: The SQL is written once here and both backends emit their impl from it with their own uuid codec

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The athlete's check-in cadence (carnet#831).
//!
//! Written when the athlete answers the onboarding question — on the web, on
//! the phone or in the messaging intake — and when they change it in
//! Settings. No row is an athlete who never chose, which reads as never
//! checked in on, exactly like `off`.
//!
//! Every statement is keyed by `tenant_id`, and by `user_id` wherever it
//! reads or writes one athlete's choice.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{CheckInCadence, TenantId};
use uuid::Uuid;

/// An athlete of a tenant who chose a cadence that posts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckInSubscriber {
    /// The athlete.
    pub user_id: Uuid,
    /// How often they asked to be checked in on — never `Off`.
    pub cadence: CheckInCadence,
}

/// The check-in cadence each athlete chose.
#[async_trait]
pub trait CheckInPreferenceRepository: Send + Sync {
    /// The cadence `user_id` chose in `tenant_id`, or `None` when they never
    /// chose one.
    async fn check_in_cadence(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
    ) -> AppResult<Option<CheckInCadence>>;

    /// Store `cadence` as `user_id`'s choice in `tenant_id`, made at `at`.
    async fn set_check_in_cadence(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        cadence: CheckInCadence,
        at: DateTime<Utc>,
    ) -> AppResult<()>;

    /// The athletes of `tenant_id` whose cadence posts — every choice but
    /// `off`.
    async fn check_in_subscribers(&self, tenant_id: TenantId) -> AppResult<Vec<CheckInSubscriber>>;
}

pub(crate) const CHECK_IN_CADENCE_SQL: &str = r"
            SELECT cadence FROM user_check_in_preferences
            WHERE tenant_id = $1 AND user_id = $2
            ";

/// `excluded` names the row the insert proposed on both engines.
pub(crate) const SET_CHECK_IN_CADENCE_SQL: &str = r"
            INSERT INTO user_check_in_preferences (tenant_id, user_id, cadence, updated_at_ms)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (tenant_id, user_id) DO UPDATE SET
                cadence = excluded.cadence,
                updated_at_ms = excluded.updated_at_ms
            ";

pub(crate) const CHECK_IN_SUBSCRIBERS_SQL: &str = r"
            SELECT user_id, cadence FROM user_check_in_preferences
            WHERE tenant_id = $1 AND cadence <> 'off'
            ORDER BY user_id
            ";

/// A stored cadence as the choice it records.
///
/// # Errors
/// Returns a database error for a spelling the schema's check refuses, which
/// only a hand-edited row can hold.
pub(crate) fn cadence_from_column(stored: &str) -> AppResult<CheckInCadence> {
    CheckInCadence::parse(stored).ok_or_else(|| {
        AppError::database(format!(
            "user_check_in_preferences.cadence {stored:?} is not a cadence"
        ))
    })
}

/// Emit the whole [`CheckInPreferenceRepository`] implementation for one
/// backend type. `$ids` is that backend's uuid codec
/// ([`crate::repositories::uuid_columns`]): `TEXT` ids on `SQLite`, native
/// `uuid` on Postgres.
macro_rules! impl_check_in_preference_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl CheckInPreferenceRepository for $ty {
            async fn check_in_cadence(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
            ) -> AppResult<Option<CheckInCadence>> {
                let stored: Option<String> = sqlx::query_scalar(CHECK_IN_CADENCE_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read the check-in cadence: {e}"))
                    })?;
                stored.as_deref().map(cadence_from_column).transpose()
            }

            async fn set_check_in_cadence(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                cadence: CheckInCadence,
                at: DateTime<Utc>,
            ) -> AppResult<()> {
                sqlx::query(SET_CHECK_IN_CADENCE_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .bind(cadence.as_str())
                    .bind(at.timestamp_millis())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store the check-in cadence: {e}"))
                    })?;
                Ok(())
            }

            async fn check_in_subscribers(
                &self,
                tenant_id: TenantId,
            ) -> AppResult<Vec<CheckInSubscriber>> {
                let rows = sqlx::query(CHECK_IN_SUBSCRIBERS_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list check-in subscribers: {e}"))
                    })?;
                rows.iter()
                    .map(|row| {
                        let cadence: String = row.try_get("cadence").map_err(|e| {
                            AppError::database(format!("Invalid check-in cadence column: {e}"))
                        })?;
                        Ok(CheckInSubscriber {
                            user_id: $ids::read(row, "user_id")?,
                            cadence: cadence_from_column(&cadence)?,
                        })
                    })
                    .collect()
            }
        }
    };
}
pub(crate) use impl_check_in_preference_repository;
