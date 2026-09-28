// ABOUTME: The one body of the system-settings key/value store, emitted for each backend
// ABOUTME: Shared statements and row decoder, so a setting reads and writes the same on SQLite and Postgres

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};

use crate::database::system_settings::SystemSetting;

/// Read one setting by key.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, so one statement serves both backends and cannot drift between
/// them.
pub(crate) const GET_SYSTEM_SETTING_SQL: &str = r"
            SELECT key, value, description, updated_at
            FROM system_settings
            WHERE key = $1
            ";

/// Insert a setting, or overwrite its value when the key exists.
pub(crate) const SET_SYSTEM_SETTING_SQL: &str = r"
            INSERT INTO system_settings (key, value, created_at, updated_at)
            VALUES ($1, $2, $3, $3)
            ON CONFLICT (key) DO UPDATE SET
                value = $2,
                updated_at = $3
            ";

/// Decode one `system_settings` row via `try_get` only, so a corrupt row is a
/// recoverable error naming the column rather than a panic or a plausible
/// default. `updated_at` is RFC 3339 text on `SQLite` and a TIMESTAMPTZ on
/// Postgres; sqlx decodes either into the same instant.
///
/// # Errors
/// Returns a database error naming the column that cannot be decoded.
pub(crate) fn system_setting_from_row<R>(row: &R) -> AppResult<SystemSetting>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column =
        |name: &str, e: sqlx::Error| AppError::database(format!("system_settings.{name}: {e}"));
    Ok(SystemSetting {
        key: row.try_get("key").map_err(|e| column("key", e))?,
        value: row.try_get("value").map_err(|e| column("value", e))?,
        description: row
            .try_get("description")
            .map_err(|e| column("description", e))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|e| column("updated_at", e))?,
    })
}

/// Emit the system-settings operations for one backend type. The body is
/// written once here; each backend invokes it with its own type, and sqlx
/// resolves the driver from `self.pool()` per expansion.
macro_rules! impl_system_settings {
    ($ty:ty) => {
        impl $ty {
            /// Get a system setting by key.
            ///
            /// # Errors
            ///
            /// Returns an error if the query fails or the row cannot be decoded.
            pub async fn get_system_setting(&self, key: &str) -> AppResult<Option<SystemSetting>> {
                let row = sqlx::query(GET_SYSTEM_SETTING_SQL)
                    .bind(key)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get system setting: {e}"))
                    })?;
                row.map(|row| system_setting_from_row(&row)).transpose()
            }

            /// Set a system setting value (upsert).
            ///
            /// # Errors
            ///
            /// Returns an error if the database operation fails.
            pub async fn set_system_setting(&self, key: &str, value: &str) -> AppResult<()> {
                sqlx::query(SET_SYSTEM_SETTING_SQL)
                    .bind(key)
                    .bind(value)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to set system setting: {e}"))
                    })?;
                Ok(())
            }

            /// Whether auto-approval is enabled in the database.
            ///
            /// Returns `Some(true/false)` if explicitly set in the database, or
            /// `None` if no setting exists (the caller uses the config default).
            ///
            /// # Errors
            ///
            /// Returns an error if the database query fails.
            pub async fn is_auto_approval_enabled(&self) -> AppResult<Option<bool>> {
                Ok(self
                    .get_system_setting(SETTING_AUTO_APPROVAL_ENABLED)
                    .await?
                    .map(|setting| setting.value.eq_ignore_ascii_case("true")))
            }

            /// Set the auto-approval enabled state.
            ///
            /// # Errors
            ///
            /// Returns an error if the database operation fails.
            pub async fn set_auto_approval_enabled(&self, enabled: bool) -> AppResult<()> {
                self.set_system_setting(
                    SETTING_AUTO_APPROVAL_ENABLED,
                    if enabled { "true" } else { "false" },
                )
                .await
            }
        }
    };
}
pub(crate) use impl_system_settings;
