// ABOUTME: Repository trait, statements and shared body for the unit system each athlete chose in Settings
// ABOUTME: Metric unless they chose imperial; emitted per backend with its uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Unit preferences (carnet#835, carnet#1044).
//!
//! The unit system an athlete chose in Settings, stored in the `preference`
//! column. A user with no row, or a row still holding `automatic` from before
//! carnet#1044, reads metric. The table's `provider_units`, `provider` and
//! `device_locale` columns are not read or written: units are never
//! inferred from the provider or the device.

use std::fmt::Display;

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::UnitSystem;
use uuid::Uuid;

/// Read and write the unit system each athlete chose.
///
/// The table carries no `tenant_id`; every statement is scoped by `user_id`:
/// the units are the person's, whichever tenant they are signed in to.
#[async_trait]
pub trait UnitPreferencesRepository: Send + Sync {
    /// The athlete's chosen system, or metric when they chose none.
    async fn get_units(&self, user_id: Uuid) -> AppResult<UnitSystem>;

    /// Store the athlete's Settings choice.
    async fn set_units(&self, user_id: Uuid, units: UnitSystem) -> AppResult<()>;
}

/// The athlete's choice.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, so one statement serves both backends. The id binds through the
/// backend's uuid codec (see [`super::uuid_columns`]).
pub(crate) const GET_UNITS_SQL: &str = r"
            SELECT preference
            FROM user_unit_preferences
            WHERE user_id = $1
            ";

/// Insert the athlete's row with their choice, or replace the choice.
pub(crate) const UPSERT_UNITS_SQL: &str = r"
            INSERT INTO user_unit_preferences (user_id, preference, updated_at)
            VALUES ($1, $2, $3)
            ON CONFLICT (user_id) DO UPDATE SET
                preference = EXCLUDED.preference,
                updated_at = EXCLUDED.updated_at
            ";

/// The error for a read or write of this table that failed.
pub(crate) fn unit_preferences_error(action: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to {action} unit preferences: {e}"))
}

/// Decode a stored choice. Anything but `imperial` — `metric`, the
/// `automatic` rows written before carnet#1044, a value from a newer build —
/// reads as metric.
pub(crate) fn decode_units(stored: &str) -> UnitSystem {
    UnitSystem::parse(stored).unwrap_or_default()
}

/// Emit the whole [`UnitPreferencesRepository`] implementation for one
/// backend type. `$ids` is the codec in [`super::uuid_columns`] for how that
/// backend's `user_id` column binds.
macro_rules! impl_unit_preferences_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl UnitPreferencesRepository for $ty {
            async fn get_units(&self, user_id: Uuid) -> AppResult<UnitSystem> {
                let row = sqlx::query(GET_UNITS_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| unit_preferences_error("read", e))?;
                let Some(row) = row else {
                    return Ok(UnitSystem::default());
                };
                let stored: String = row
                    .try_get("preference")
                    .map_err(|e| unit_preferences_error("decode", e))?;
                Ok(decode_units(&stored))
            }

            async fn set_units(&self, user_id: Uuid, units: UnitSystem) -> AppResult<()> {
                sqlx::query(UPSERT_UNITS_SQL)
                    .bind($ids::bind(user_id))
                    .bind(units.as_str())
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| unit_preferences_error("store", e))?;
                Ok(())
            }
        }
    };
}
pub(crate) use impl_unit_preferences_repository;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_choice_decodes() {
        assert_eq!(decode_units("imperial"), UnitSystem::Imperial);
        assert_eq!(decode_units("metric"), UnitSystem::Metric);
    }

    #[test]
    fn automatic_and_unknown_values_read_metric() {
        assert_eq!(decode_units("automatic"), UnitSystem::Metric);
        assert_eq!(decode_units("nautical"), UnitSystem::Metric);
    }
}
