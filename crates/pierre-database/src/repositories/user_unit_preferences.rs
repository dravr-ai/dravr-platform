// ABOUTME: Repository trait, statements and shared body for the inputs that decide each athlete's unit system
// ABOUTME: The Settings choice, the provider's own setting and the device locale; emitted per backend with its uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Unit preferences (carnet#835).
//!
//! What decides whether an athlete reads kilometres or miles: their explicit
//! choice in Settings, the unit setting of the provider they connected
//! (Strava's `measurement_preference`), and the locale their device reported.
//! This module only stores the three; [`pierre_core::models::resolve_units`]
//! is the one rule that turns them into a unit system.
//!
//! Each input is written by its own statement, so a Settings change never
//! clears what the provider said and a provider read never overwrites the
//! athlete's choice. A user with no row has told us nothing, and reads as
//! [`StoredUnitPreferences::default`].

use std::fmt::Display;

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{UnitPreference, UnitSystem};
use uuid::Uuid;

/// The inputs stored for one athlete.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoredUnitPreferences {
    /// The athlete's choice in Settings.
    pub preference: UnitPreference,
    /// The unit system the athlete set on their provider, when one was read.
    pub provider_units: Option<UnitSystem>,
    /// The provider [`Self::provider_units`] was read from.
    pub provider: Option<String>,
    /// The BCP 47 locale tag the athlete's device last reported (`en-US`).
    pub device_locale: Option<String>,
}

/// Read and write the inputs to an athlete's unit system.
///
/// The table carries no `tenant_id`; every statement is scoped by `user_id`:
/// the units are the person's, whichever tenant they are signed in to.
#[async_trait]
pub trait UnitPreferencesRepository: Send + Sync {
    /// The stored inputs, or the defaults when nothing is stored.
    async fn get_unit_preferences(&self, user_id: Uuid) -> AppResult<StoredUnitPreferences>;

    /// Store the athlete's Settings choice, leaving the other inputs as they are.
    async fn set_unit_preference(&self, user_id: Uuid, preference: UnitPreference)
        -> AppResult<()>;

    /// Store the unit system read from `provider`'s own athlete profile.
    async fn set_provider_units(
        &self,
        user_id: Uuid,
        provider: &str,
        units: UnitSystem,
    ) -> AppResult<()>;

    /// Store the locale tag the athlete's device reported.
    async fn set_device_locale(&self, user_id: Uuid, device_locale: &str) -> AppResult<()>;
}

/// The athlete's row.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, so one statement serves both backends. The id binds through the
/// backend's uuid codec (see [`super::uuid_columns`]).
pub(crate) const GET_UNIT_PREFERENCES_SQL: &str = r"
            SELECT preference, provider_units, provider, device_locale
            FROM user_unit_preferences
            WHERE user_id = $1
            ";

/// Insert the athlete's row with their choice, or replace the choice alone.
pub(crate) const UPSERT_UNIT_PREFERENCE_SQL: &str = r"
            INSERT INTO user_unit_preferences (user_id, preference, updated_at)
            VALUES ($1, $2, $3)
            ON CONFLICT (user_id) DO UPDATE SET
                preference = EXCLUDED.preference,
                updated_at = EXCLUDED.updated_at
            ";

/// Insert the athlete's row with the provider's setting, or replace that alone.
pub(crate) const UPSERT_PROVIDER_UNITS_SQL: &str = r"
            INSERT INTO user_unit_preferences (user_id, provider_units, provider, updated_at)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (user_id) DO UPDATE SET
                provider_units = EXCLUDED.provider_units,
                provider = EXCLUDED.provider,
                updated_at = EXCLUDED.updated_at
            ";

/// Insert the athlete's row with the device locale, or replace that alone.
pub(crate) const UPSERT_DEVICE_LOCALE_SQL: &str = r"
            INSERT INTO user_unit_preferences (user_id, device_locale, updated_at)
            VALUES ($1, $2, $3)
            ON CONFLICT (user_id) DO UPDATE SET
                device_locale = EXCLUDED.device_locale,
                updated_at = EXCLUDED.updated_at
            ";

/// The error for a read or write of this table that failed.
pub(crate) fn unit_preferences_error(action: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to {action} unit preferences: {e}"))
}

/// Decode a stored row. A value this build does not know — written by a
/// newer one — reads as nothing known rather than failing the read.
pub(crate) fn decode_unit_preferences(
    preference: &str,
    provider_units: Option<&str>,
    provider: Option<String>,
    device_locale: Option<String>,
) -> StoredUnitPreferences {
    StoredUnitPreferences {
        preference: UnitPreference::parse(preference).unwrap_or_default(),
        provider_units: provider_units.and_then(UnitSystem::parse),
        provider,
        device_locale,
    }
}

/// Emit the whole [`UnitPreferencesRepository`] implementation for one
/// backend type. `$ids` is the codec in [`super::uuid_columns`] for how that
/// backend's `user_id` column binds.
macro_rules! impl_unit_preferences_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl UnitPreferencesRepository for $ty {
            async fn get_unit_preferences(
                &self,
                user_id: Uuid,
            ) -> AppResult<StoredUnitPreferences> {
                let row = sqlx::query(GET_UNIT_PREFERENCES_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| unit_preferences_error("read", e))?;
                let Some(row) = row else {
                    return Ok(StoredUnitPreferences::default());
                };
                let decode = |e| unit_preferences_error("decode", e);
                let preference: String = row.try_get("preference").map_err(decode)?;
                let provider_units: Option<String> =
                    row.try_get("provider_units").map_err(decode)?;
                Ok(decode_unit_preferences(
                    &preference,
                    provider_units.as_deref(),
                    row.try_get("provider").map_err(decode)?,
                    row.try_get("device_locale").map_err(decode)?,
                ))
            }

            async fn set_unit_preference(
                &self,
                user_id: Uuid,
                preference: UnitPreference,
            ) -> AppResult<()> {
                sqlx::query(UPSERT_UNIT_PREFERENCE_SQL)
                    .bind($ids::bind(user_id))
                    .bind(preference.as_str())
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| unit_preferences_error("store", e))?;
                Ok(())
            }

            async fn set_provider_units(
                &self,
                user_id: Uuid,
                provider: &str,
                units: UnitSystem,
            ) -> AppResult<()> {
                sqlx::query(UPSERT_PROVIDER_UNITS_SQL)
                    .bind($ids::bind(user_id))
                    .bind(units.as_str())
                    .bind(provider)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| unit_preferences_error("store", e))?;
                Ok(())
            }

            async fn set_device_locale(&self, user_id: Uuid, device_locale: &str) -> AppResult<()> {
                sqlx::query(UPSERT_DEVICE_LOCALE_SQL)
                    .bind($ids::bind(user_id))
                    .bind(device_locale)
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
    fn a_stored_row_decodes_every_input() {
        let stored = decode_unit_preferences(
            "imperial",
            Some("metric"),
            Some("strava".to_owned()),
            Some("en-US".to_owned()),
        );
        assert_eq!(stored.preference, UnitPreference::Imperial);
        assert_eq!(stored.provider_units, Some(UnitSystem::Metric));
        assert_eq!(stored.provider.as_deref(), Some("strava"));
        assert_eq!(stored.device_locale.as_deref(), Some("en-US"));
    }

    #[test]
    fn an_unknown_stored_value_reads_as_nothing_known() {
        let stored = decode_unit_preferences("nautical", Some("cubits"), None, None);
        assert_eq!(stored.preference, UnitPreference::Automatic);
        assert_eq!(stored.provider_units, None);
    }
}
