// ABOUTME: The one body of the system-settings key/value store, emitted for each backend
// ABOUTME: Shared statements and row decoder, so a setting reads and writes the same on SQLite and Postgres

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Duration, Utc};
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

/// Overwrite a setting only while its `updated_at` still holds the value the
/// writer read. A row someone else changed in between matches nothing.
pub(crate) const UPDATE_SYSTEM_SETTING_IF_UNCHANGED_SQL: &str = r"
            UPDATE system_settings
            SET value = $2, updated_at = $3
            WHERE key = $1 AND updated_at = $4
            ";

/// Insert a setting only while no row holds the key.
pub(crate) const INSERT_SYSTEM_SETTING_IF_ABSENT_SQL: &str = r"
            INSERT INTO system_settings (key, value, created_at, updated_at)
            VALUES ($1, $2, $3, $3)
            ON CONFLICT (key) DO NOTHING
            ";

/// The `updated_at` a compare-and-set writes, so it reads back as the exact
/// instant the caller is handed.
///
/// Whole microseconds, because a Postgres `TIMESTAMPTZ` keeps no more: a
/// nanosecond stamp handed to the caller would never match the stored row and
/// their next write would be refused as stale. Always past `expected`, because
/// the stamp is the concurrency token: on a clock running behind the instance
/// that wrote `expected`, or in the same microsecond, a write would otherwise
/// leave the token unmoved and a second writer holding it would still pass.
pub(crate) fn next_setting_stamp(
    now: DateTime<Utc>,
    expected: Option<DateTime<Utc>>,
) -> DateTime<Utc> {
    use chrono::SubsecRound;

    let now = now.trunc_subsecs(6);
    match expected {
        Some(prior) if now <= prior => prior.trunc_subsecs(6) + Duration::microseconds(1),
        _ => now,
    }
}

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

/// The operator's key/value settings, reached through the registry.
///
/// The `dyn` seam for code that takes repository traits rather than the
/// concrete backend: `pierre_auth`'s connect order (carnet#1472), read beside
/// the token and connection repositories, and the backfill notifier, which
/// holds only the registry. A missing row is `None`; every reader chooses its
/// own default.
#[async_trait::async_trait]
pub trait SystemSettingsRepository: Send + Sync {
    /// Read one setting by key.
    ///
    /// # Errors
    /// Returns an error if the query fails or the row cannot be decoded.
    async fn get_setting(&self, key: &str) -> AppResult<Option<SystemSetting>>;

    /// Write one setting (upsert).
    ///
    /// # Errors
    /// Returns an error if the write fails.
    async fn set_setting(&self, key: &str, value: &str) -> AppResult<()>;
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

            /// Write a setting only when the row is as the caller last read it:
            /// `expected` is the `updated_at` it read, or `None` to create the
            /// row only while no row holds the key.
            ///
            /// Returns the new `updated_at` when the write landed, `None` when
            /// the row changed since the read (or, for `None`, already exists).
            /// The returned stamp reads back exactly on either backend, so it
            /// can be the `expected` of the next write.
            ///
            /// # Errors
            ///
            /// Returns an error if the database operation fails.
            pub async fn compare_and_set_system_setting(
                &self,
                key: &str,
                value: &str,
                expected: Option<DateTime<Utc>>,
            ) -> AppResult<Option<DateTime<Utc>>> {
                let stamp = next_setting_stamp(Utc::now(), expected);
                let query = match expected {
                    Some(prior) => sqlx::query(UPDATE_SYSTEM_SETTING_IF_UNCHANGED_SQL)
                        .bind(key)
                        .bind(value)
                        .bind(stamp)
                        .bind(prior),
                    None => sqlx::query(INSERT_SYSTEM_SETTING_IF_ABSENT_SQL)
                        .bind(key)
                        .bind(value)
                        .bind(stamp),
                };
                let result = query.execute(self.pool()).await.map_err(|e| {
                    AppError::database(format!("Failed to compare-and-set system setting: {e}"))
                })?;
                Ok((result.rows_affected() > 0).then_some(stamp))
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

        #[async_trait::async_trait]
        impl $crate::repositories::system_settings::SystemSettingsRepository for $ty {
            async fn get_setting(&self, key: &str) -> AppResult<Option<SystemSetting>> {
                self.get_system_setting(key).await
            }

            async fn set_setting(&self, key: &str, value: &str) -> AppResult<()> {
                self.set_system_setting(key, value).await
            }
        }
    };
}
pub(crate) use impl_system_settings;

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn a_stamp_is_whole_microseconds_and_always_moves_past_the_token() {
        let at = |nanos: u32| Utc.timestamp_opt(1_700_000_000, nanos).single().unwrap();

        assert_eq!(next_setting_stamp(at(1_234_567), None), at(1_234_000));

        // Later than the token: the clock's own microsecond.
        assert_eq!(
            next_setting_stamp(at(9_000_500), Some(at(1_000_000))),
            at(9_000_000)
        );
        // The same microsecond, and a clock behind the instance that wrote the
        // token: the token still moves.
        assert_eq!(
            next_setting_stamp(at(1_000_400), Some(at(1_000_000))),
            at(1_001_000)
        );
        assert_eq!(
            next_setting_stamp(at(500_000), Some(at(1_000_000))),
            at(1_001_000)
        );
    }
}
