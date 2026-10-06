// ABOUTME: Shared statements and bodies for the preference columns on the users row
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

/// Record which exposure notice of a provider the user accepted, and when. A
/// later acceptance of a newer version replaces the row, and an acceptance
/// clears an earlier withdrawal. `excluded` and `CURRENT_TIMESTAMP` are
/// spellings both engines accept.
pub(crate) const SET_PROVIDER_TERMS_SQL: &str = r"
        INSERT INTO provider_terms_consents (user_id, provider, version, consented_at)
        VALUES ($1, $2, $3, CURRENT_TIMESTAMP)
        ON CONFLICT (user_id, provider) DO UPDATE SET
            version = excluded.version,
            consented_at = excluded.consented_at,
            withdrawn_at = NULL
        ";

/// Read the notice version the user accepted for a provider and has not
/// withdrawn; there is no row until they accept one, and a withdrawn row
/// reads as none.
pub(crate) const GET_PROVIDER_TERMS_SQL: &str = r"
        SELECT version FROM provider_terms_consents
        WHERE user_id = $1 AND provider = $2 AND withdrawn_at IS NULL
        ";

/// Stamp the user's standing acceptance of a provider's notice as withdrawn.
/// Touches nothing when there is none: a withdrawal is never re-dated.
pub(crate) const WITHDRAW_PROVIDER_TERMS_SQL: &str = r"
        UPDATE provider_terms_consents SET withdrawn_at = CURRENT_TIMESTAMP
        WHERE user_id = $1 AND provider = $2 AND withdrawn_at IS NULL
        ";

/// Set the user's preferred locale.
pub(crate) const SET_LOCALE_SQL: &str = "UPDATE users SET locale = $1 WHERE id = $2";

/// Set the coaching persona, persisted as `snake_case` enum text.
pub(crate) const SET_COACHING_PERSONA_SQL: &str =
    "UPDATE users SET coaching_persona = $1 WHERE id = $2";

/// Set whether the user manages a coaching roster.
pub(crate) const SET_MANAGES_ROSTER_SQL: &str =
    "UPDATE users SET manages_roster = $1 WHERE id = $2";

/// Set `manages_roster` as an operator decision, with who made it and when:
/// a grant stamps both, a revoke clears both.
pub(crate) const SET_ROSTER_BY_OPERATOR_SQL: &str = r"
        UPDATE users SET
            manages_roster = $1,
            manages_roster_granted_at = $2,
            manages_roster_granted_by = $3
        WHERE id = $4
        ";

/// Take back a `manages_roster` grant only when no operator made it. The flag
/// values are bound rather than written as literals: `SQLite` stores them as
/// integers and Postgres as booleans, and a bound `bool` is either.
pub(crate) const REVOKE_EARNED_ROSTER_SQL: &str = r"
        UPDATE users SET manages_roster = $1
        WHERE id = $2 AND manages_roster = $3 AND manages_roster_granted_at IS NULL
        ";

/// Read who granted the user's `manages_roster` as an operator, and when.
pub(crate) const GET_ROSTER_OPERATOR_GRANT_SQL: &str =
    "SELECT manages_roster_granted_at, manages_roster_granted_by FROM users WHERE id = $1";

/// Set the user's IANA timezone.
pub(crate) const SET_TIMEZONE_SQL: &str = "UPDATE users SET timezone = $1 WHERE id = $2";

/// Pin, or clear, the user's colour scheme.
pub(crate) const SET_THEME_SQL: &str = "UPDATE users SET theme = $1 WHERE id = $2";

/// Emit every preference write for one backend.
///
/// `$db` is the sqlx database type the pool is parameterised on; `$ids` is the
/// backend's codec in [`super::uuid_columns`], whose `bind` turns a `Uuid`
/// into whatever that backend's `users.id` column accepts and whose `read_opt`
/// reads one back.
///
/// The bodies are written once here. Each backend module invokes the macro,
/// and sqlx resolves the driver from the pool type at that expansion.
macro_rules! impl_user_preferences {
    ($db:ty, $ids:ident) => {
        /// Turn "no row matched" into the `NotFound` the callers contract on.
        fn ensure_updated(rows_affected: u64, user_id: Uuid) -> AppResult<()> {
            (rows_affected > 0)
                .ok_or_else(|| AppError::not_found(format!("User with ID: {user_id}")))
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
                .bind($ids::bind(user_id))
                .execute(pool)
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to update analytics consent: {e}"))
                })?;

            ensure_updated(result.rows_affected(), user_id)
        }

        /// Record that the user accepted `provider`'s exposure notice
        /// `version`, stamping the time.
        ///
        /// The record belongs to the account, not to a provider session: it is
        /// the account's answer to the notice, so a disconnect leaves it and a
        /// reconnect reads it.
        ///
        /// # Errors
        ///
        /// Returns an error if the database write fails, including an unknown
        /// user (the row references `users`).
        pub async fn record_provider_terms(
            pool: &Pool<$db>,
            user_id: Uuid,
            provider: &str,
            version: &str,
        ) -> AppResult<()> {
            sqlx::query(SET_PROVIDER_TERMS_SQL)
                .bind($ids::bind(user_id))
                .bind(provider)
                .bind(version)
                .execute(pool)
                .await
                .map_err(|e| {
                    AppError::database(format!(
                        "Failed to record the {provider} notice consent: {e}"
                    ))
                })?;
            Ok(())
        }

        /// Withdraw the user's standing acceptance of `provider`'s notice.
        ///
        /// The row stays, with the version last accepted and the time of the
        /// withdrawal; the acceptance reads as none from then on, until the
        /// user accepts again.
        ///
        /// Returns whether an acceptance was standing.
        ///
        /// # Errors
        ///
        /// Returns an error if the database write fails.
        pub async fn withdraw_provider_terms(
            pool: &Pool<$db>,
            user_id: Uuid,
            provider: &str,
        ) -> AppResult<bool> {
            let result = sqlx::query(WITHDRAW_PROVIDER_TERMS_SQL)
                .bind($ids::bind(user_id))
                .bind(provider)
                .execute(pool)
                .await
                .map_err(|e| {
                    AppError::database(format!(
                        "Failed to withdraw the {provider} notice consent: {e}"
                    ))
                })?;
            Ok(result.rows_affected() > 0)
        }

        /// The notice version the user accepted for `provider` and has not
        /// withdrawn, or `None` when no acceptance stands.
        ///
        /// # Errors
        ///
        /// Returns an error if the database query fails.
        pub async fn provider_terms_version(
            pool: &Pool<$db>,
            user_id: Uuid,
            provider: &str,
        ) -> AppResult<Option<String>> {
            sqlx::query_scalar(GET_PROVIDER_TERMS_SQL)
                .bind($ids::bind(user_id))
                .bind(provider)
                .fetch_optional(pool)
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to read the {provider} notice consent: {e}"))
                })
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
                .bind($ids::bind(user_id))
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
                .bind($ids::bind(user_id))
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
                .bind($ids::bind(user_id))
                .execute(pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to set manages_roster: {e}")))?;

            ensure_updated(result.rows_affected(), user_id)
        }

        /// Grant or revoke `manages_roster` as an operator. A grant records
        /// `operator` and the time, which the `TrainingPeaks` reconciler reads
        /// as a grant it must leave alone; a revoke clears both.
        ///
        /// # Errors
        ///
        /// Returns an error if the user is not found or the database update
        /// fails.
        pub async fn set_manages_roster_by_operator(
            pool: &Pool<$db>,
            user_id: Uuid,
            manages_roster: bool,
            operator: Option<Uuid>,
        ) -> AppResult<()> {
            let granted_at = manages_roster.then(Utc::now);
            let granted_by = operator.filter(|_| manages_roster);
            let result = sqlx::query(SET_ROSTER_BY_OPERATOR_SQL)
                .bind(manages_roster)
                .bind(granted_at)
                .bind($ids::bind_opt(granted_by))
                .bind($ids::bind(user_id))
                .execute(pool)
                .await
                .map_err(|e| {
                    AppError::database(format!("Failed to set manages_roster by operator: {e}"))
                })?;

            ensure_updated(result.rows_affected(), user_id)
        }

        /// Take back a `manages_roster` grant no operator made. Returns whether
        /// a grant was taken back: `false` for a user without one, one an
        /// operator made, or no user at all.
        ///
        /// # Errors
        ///
        /// Returns an error if the database update fails.
        pub async fn revoke_earned_manages_roster(
            pool: &Pool<$db>,
            user_id: Uuid,
        ) -> AppResult<bool> {
            let result = sqlx::query(REVOKE_EARNED_ROSTER_SQL)
                .bind(false)
                .bind($ids::bind(user_id))
                .bind(true)
                .execute(pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to revoke manages_roster: {e}")))?;
            Ok(result.rows_affected() > 0)
        }

        /// The operator grant behind the user's `manages_roster`, `None` when
        /// no operator made one.
        ///
        /// # Errors
        ///
        /// Returns an error if the user is not found or the database query
        /// fails.
        pub async fn manages_roster_operator_grant(
            pool: &Pool<$db>,
            user_id: Uuid,
        ) -> AppResult<Option<OperatorRosterGrant>> {
            let row = sqlx::query(GET_ROSTER_OPERATOR_GRANT_SQL)
                .bind($ids::bind(user_id))
                .fetch_optional(pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to read the roster grant: {e}")))?
                .ok_or_else(|| AppError::not_found(format!("User with ID: {user_id}")))?;
            let granted_at: Option<DateTime<Utc>> =
                row.try_get("manages_roster_granted_at").map_err(|e| {
                    AppError::database(format!("Failed to read manages_roster_granted_at: {e}"))
                })?;
            let granted_by = $ids::read_opt(&row, "manages_roster_granted_by")?;
            Ok(granted_at.map(|granted_at| OperatorRosterGrant {
                granted_at,
                granted_by,
            }))
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
                .bind($ids::bind(user_id))
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
                .bind($ids::bind(user_id))
                .execute(pool)
                .await
                .map_err(|e| AppError::database(format!("Failed to set theme: {e}")))?;

            ensure_updated(result.rows_affected(), user_id)
        }
    };
}
pub(crate) use impl_user_preferences;
