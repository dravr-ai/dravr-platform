// ABOUTME: Repository trait, statements and shared body for the calendar an athlete chose for Dravr's pushes
// ABOUTME: Tenant-scoped rows keyed by (tenant, athlete) with who recorded the choice; emitted per backend by macro
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Calendar preferences (carnet#1393).
//!
//! An athlete who holds several calendar-writing connections names the one
//! Dravr's plan and workout pushes go to. The push target reads it before the
//! push ledger and the provider ranking. One row per athlete per tenant: the
//! tenant their connections and ledger live under, which every statement
//! filters on together with the athlete.

use std::fmt::Display;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

/// The calendar an athlete chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarPreference {
    /// The backend that writes the calendar, as the push ledger names it.
    pub provider: String,
    /// Who recorded it: the athlete, or a group's human coach writing their
    /// calendar. `None` once that account is deleted.
    pub set_by: Option<Uuid>,
    /// When it was recorded.
    pub updated_at: DateTime<Utc>,
}

/// Read and write the calendar an athlete chose.
#[async_trait]
pub trait CalendarPreferenceRepository: Send + Sync {
    /// The calendar `user_id` chose under `tenant`, if any.
    async fn get_calendar_preference(
        &self,
        tenant: TenantId,
        user_id: Uuid,
    ) -> AppResult<Option<CalendarPreference>>;

    /// Record `provider` as the calendar `user_id` chose under `tenant`,
    /// replacing any earlier choice. `set_by` is who recorded it.
    async fn set_calendar_preference(
        &self,
        tenant: TenantId,
        user_id: Uuid,
        provider: &str,
        set_by: Uuid,
        at: DateTime<Utc>,
    ) -> AppResult<()>;

    /// Forget the choice, so the next push asks again when several calendars
    /// qualify. Returns whether there was one.
    async fn clear_calendar_preference(&self, tenant: TenantId, user_id: Uuid) -> AppResult<bool>;
}

/// Record a choice, or replace the athlete's earlier one.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, so one statement serves both backends. The user ids bind
/// through the backend's uuid codec (see [`super::uuid_columns`]); the
/// tenant is `TEXT` on both.
pub(crate) const SET_CALENDAR_PREFERENCE_SQL: &str = r"
            INSERT INTO calendar_preferences (tenant_id, user_id, provider, set_by, updated_at)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (tenant_id, user_id) DO UPDATE SET
                provider = EXCLUDED.provider,
                set_by = EXCLUDED.set_by,
                updated_at = EXCLUDED.updated_at
            ";

/// The athlete's choice under one tenant.
pub(crate) const GET_CALENDAR_PREFERENCE_SQL: &str = r"
            SELECT provider, set_by, updated_at
            FROM calendar_preferences
            WHERE tenant_id = $1 AND user_id = $2
            ";

/// Forget the athlete's choice under one tenant.
pub(crate) const CLEAR_CALENDAR_PREFERENCE_SQL: &str = r"
            DELETE FROM calendar_preferences
            WHERE tenant_id = $1 AND user_id = $2
            ";

/// The error for a read or write of this table that failed.
pub(crate) fn preference_error(action: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to {action} calendar preference: {e}"))
}

/// Emit the whole [`CalendarPreferenceRepository`] implementation for one
/// backend type. `$ids` is the codec in [`super::uuid_columns`] for how that
/// backend's user id columns bind and read.
macro_rules! impl_calendar_preference_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl CalendarPreferenceRepository for $ty {
            async fn get_calendar_preference(
                &self,
                tenant: TenantId,
                user_id: Uuid,
            ) -> AppResult<Option<CalendarPreference>> {
                let row = sqlx::query(GET_CALENDAR_PREFERENCE_SQL)
                    .bind(tenant.to_string())
                    .bind($ids::bind(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| preference_error("read", e))?;
                row.map(|row| {
                    Ok(CalendarPreference {
                        provider: row
                            .try_get("provider")
                            .map_err(|e| preference_error("decode", e))?,
                        set_by: $ids::read_opt(&row, "set_by")?,
                        updated_at: row
                            .try_get("updated_at")
                            .map_err(|e| preference_error("decode", e))?,
                    })
                })
                .transpose()
            }

            async fn set_calendar_preference(
                &self,
                tenant: TenantId,
                user_id: Uuid,
                provider: &str,
                set_by: Uuid,
                at: DateTime<Utc>,
            ) -> AppResult<()> {
                sqlx::query(SET_CALENDAR_PREFERENCE_SQL)
                    .bind(tenant.to_string())
                    .bind($ids::bind(user_id))
                    .bind(provider)
                    .bind($ids::bind(set_by))
                    .bind(at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| preference_error("store", e))?;
                Ok(())
            }

            async fn clear_calendar_preference(
                &self,
                tenant: TenantId,
                user_id: Uuid,
            ) -> AppResult<bool> {
                let done = sqlx::query(CLEAR_CALENDAR_PREFERENCE_SQL)
                    .bind(tenant.to_string())
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| preference_error("delete", e))?;
                Ok(done.rows_affected() > 0)
            }
        }
    };
}
pub(crate) use impl_calendar_preference_repository;
