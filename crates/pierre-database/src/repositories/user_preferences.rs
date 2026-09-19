// ABOUTME: Shared statements and body for the single-column preference writes on the users row
// ABOUTME: One SQL text per preference; each backend shell supplies only how it binds a user id

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Preference writes, written once.
//!
//! Each of these sets exactly one column on `users` and reports a missing row
//! as `NotFound` rather than a silent no-op — a preference the client believes
//! it saved and the server quietly dropped is worse than an error.
//!
//! The two backends differ in one respect only: `users.id` is a `uuid` column
//! on Postgres and `TEXT` on `SQLite`, so the id is bound natively on one and
//! stringified on the other. That conversion is the macro's single argument:
//! the `bind` of the backend's codec in [`super::uuid_columns`].
//!
//! `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
//! Postgres, so one statement serves both drivers and cannot drift between
//! them.

/// Record the consent decision and the moment it was taken.
/// `CURRENT_TIMESTAMP` is the spelling both engines accept.
pub(crate) const SET_ANALYTICS_CONSENT_SQL: &str = r"
        UPDATE users SET
            analytics_consent = $1,
            analytics_consent_at = CURRENT_TIMESTAMP
        WHERE id = $2
        ";

/// Set the user's preferred locale.
pub(crate) const SET_LOCALE_SQL: &str = "UPDATE users SET locale = $1 WHERE id = $2";

/// Set the coaching persona, persisted as `snake_case` enum text.
pub(crate) const SET_COACHING_PERSONA_SQL: &str =
    "UPDATE users SET coaching_persona = $1 WHERE id = $2";

/// Set whether the user manages a coaching roster.
pub(crate) const SET_MANAGES_ROSTER_SQL: &str =
    "UPDATE users SET manages_roster = $1 WHERE id = $2";

/// Set the user's IANA timezone.
pub(crate) const SET_TIMEZONE_SQL: &str = "UPDATE users SET timezone = $1 WHERE id = $2";

/// Pin, or clear, the user's colour scheme.
pub(crate) const SET_THEME_SQL: &str = "UPDATE users SET theme = $1 WHERE id = $2";

/// Emit every preference write for one backend.
///
/// `$db` is the sqlx database type the pool is parameterised on; `$bind_id` is
/// the function turning a `Uuid` into whatever that backend's `users.id`
/// column accepts — the `bind` of its codec in [`super::uuid_columns`].
///
/// The bodies are written once here. Each backend module invokes the macro,
/// and sqlx resolves the driver from the pool type at that expansion.
macro_rules! impl_user_preferences {
    ($db:ty, $bind_id:path) => {
        /// Turn "no row matched" into the `NotFound` the callers contract on.
        fn ensure_updated(rows_affected: u64, user_id: Uuid) -> AppResult<()> {
            if rows_affected == 0 {
                return Err(AppError::not_found(format!("User with ID: {user_id}")));
            }
            Ok(())
        }

        /// Update the user's analytics-consent preference, stamping the
        /// decision time.
        ///
        /// # Errors
        ///
        /// Returns an error if the user is not found or the database update
        /// fails.
        pub async fn update_analytics_consent(
            pool: &Pool<$db>,
            user_id: Uuid,
            enabled: bool,
        ) -> AppResult<()> {
            let result = sqlx::query(SET_ANALYTICS_CONSENT_SQL)
                .bind(enabled)
                .bind($bind_id(user_id))
                .execute(pool)
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to update analytics consent: {e}"))
                })?;

            ensure_updated(result.rows_affected(), user_id)
        }

        /// Update the user's preferred locale.
        ///
        /// # Errors
        ///
        /// Returns an error if the user is not found or the database update
        /// fails.
        pub async fn update_locale(pool: &Pool<$db>, user_id: Uuid, locale: &str) -> AppResult<()> {
            let result = sqlx::query(SET_LOCALE_SQL)
                .bind(locale)
                .bind($bind_id(user_id))
                .execute(pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to update user locale: {e}")))?;

            ensure_updated(result.rows_affected(), user_id)
        }

        /// Set the user's coaching persona (output format / cadence
        /// preference).
        ///
        /// Persisted as `snake_case` enum text — the column has
        /// `NOT NULL DEFAULT 'casual'` and the application-side
        /// [`CoachingPersona`] enum is the source of truth for the allowed
        /// value set.
        ///
        /// # Errors
        ///
        /// Returns an error if the user is not found or the database update
        /// fails.
        pub async fn set_coaching_persona(
            pool: &Pool<$db>,
            user_id: Uuid,
            persona: CoachingPersona,
        ) -> AppResult<()> {
            let result = sqlx::query(SET_COACHING_PERSONA_SQL)
                .bind(persona.as_str())
                .bind($bind_id(user_id))
                .execute(pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to set coaching persona: {e}")))?;

            ensure_updated(result.rows_affected(), user_id)
        }

        /// Set whether the user manages a coaching roster.
        ///
        /// # Errors
        ///
        /// Returns an error if the user is not found or the database update
        /// fails.
        pub async fn set_manages_roster(
            pool: &Pool<$db>,
            user_id: Uuid,
            manages_roster: bool,
        ) -> AppResult<()> {
            let result = sqlx::query(SET_MANAGES_ROSTER_SQL)
                .bind(manages_roster)
                .bind($bind_id(user_id))
                .execute(pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to set manages_roster: {e}")))?;

            ensure_updated(result.rows_affected(), user_id)
        }

        /// Set the user's IANA timezone.
        ///
        /// # Errors
        ///
        /// Returns an error if the user is not found or the database update
        /// fails.
        pub async fn set_timezone(
            pool: &Pool<$db>,
            user_id: Uuid,
            timezone: &str,
        ) -> AppResult<()> {
            let result = sqlx::query(SET_TIMEZONE_SQL)
                .bind(timezone)
                .bind($bind_id(user_id))
                .execute(pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to set timezone: {e}")))?;

            ensure_updated(result.rows_affected(), user_id)
        }

        /// Pin, or clear, the user's colour scheme.
        ///
        /// `Some("light")` / `Some("dark")` pin the scheme across every
        /// device; `None` clears the pin so clients follow the operating
        /// system and server-side chart renders fall back to dark.
        ///
        /// # Errors
        ///
        /// Returns an error if the user is not found or the database update
        /// fails.
        pub async fn set_theme(
            pool: &Pool<$db>,
            user_id: Uuid,
            theme: Option<&str>,
        ) -> AppResult<()> {
            let result = sqlx::query(SET_THEME_SQL)
                .bind(theme)
                .bind($bind_id(user_id))
                .execute(pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to set theme: {e}")))?;

            ensure_updated(result.rows_affected(), user_id)
        }
    };
}
pub(crate) use impl_user_preferences;
