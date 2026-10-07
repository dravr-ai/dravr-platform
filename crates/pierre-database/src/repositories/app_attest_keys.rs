// ABOUTME: Repository trait, shared statements and body for iOS App Attest keys — register once, read, advance the counter
// ABOUTME: Every column is backend-neutral, so one SQL text per operation serves SQLite and Postgres alike

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! iOS App Attest keys (carnet#810).
//!
//! An install of the iOS app registers its key once, when its attestation
//! verifies; every later sign-in from it carries an assertion whose counter
//! must move past the stored one. The table is keyed by the key id and holds
//! no user or tenant: the key vouches for the install, whoever signs in on it
//! (see [`AppAttestKey`]).
//!
//! `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
//! Postgres. Timestamps bind as [`DateTime<Utc>`] on both: `TIMESTAMPTZ` on
//! Postgres, RFC 3339 text on `SQLite`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{AppAttestEnvironment, AppAttestKey};

/// The attested keys of iOS app installs.
#[async_trait]
pub trait AppAttestKeyRepository: Send + Sync {
    /// Register a key whose attestation verified. `false` when the key id is
    /// already registered: Apple attests a key once, so a second attestation
    /// of the same key is a replay, and the stored record is kept.
    async fn register_key(&self, key: &AppAttestKey) -> AppResult<bool>;
    /// The registered key with this id, if any.
    async fn find_key(&self, key_id: &str) -> AppResult<Option<AppAttestKey>>;
    /// Store the counter of an assertion just verified, only if it is past
    /// the stored one, in one statement so two concurrent sign-ins cannot both
    /// spend the same counter. `false` when it was not.
    async fn advance_counter(
        &self,
        key_id: &str,
        sign_count: u32,
        now: DateTime<Utc>,
    ) -> AppResult<bool>;
}

/// Register a key, keeping the first registration of a key id.
pub(crate) const REGISTER_KEY_SQL: &str = r"
            INSERT INTO app_attest_keys
                (key_id, public_key, environment, sign_count, created_at, last_used_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (key_id) DO NOTHING
            ";

/// Read a key by its id.
pub(crate) const FIND_KEY_SQL: &str = r"
            SELECT key_id, public_key, environment, sign_count, created_at, last_used_at
            FROM app_attest_keys
            WHERE key_id = $1
            ";

/// Move the counter forward; matches nothing when it would not.
pub(crate) const ADVANCE_COUNTER_SQL: &str = r"
            UPDATE app_attest_keys
            SET sign_count = $2, last_used_at = $3
            WHERE key_id = $1 AND sign_count < $2
            ";

/// Decode a key row. `try_get` throughout, never `Row::get`, so a corrupt row
/// surfaces as a recoverable error rather than a panic.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded,
/// a counter outside `u32`, or an environment other than the two stored.
pub(crate) fn key_from_row<R>(row: &R) -> AppResult<AppAttestKey>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Vec<u8>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let sign_count: i64 = row
        .try_get("sign_count")
        .map_err(|e| AppError::database(format!("Failed to get sign_count: {e}")))?;
    let environment: String = row
        .try_get("environment")
        .map_err(|e| AppError::database(format!("Failed to get environment: {e}")))?;
    Ok(AppAttestKey {
        key_id: row
            .try_get("key_id")
            .map_err(|e| AppError::database(format!("Failed to get key_id: {e}")))?,
        public_key: row
            .try_get("public_key")
            .map_err(|e| AppError::database(format!("Failed to get public_key: {e}")))?,
        environment: AppAttestEnvironment::from_stored(&environment).ok_or_else(|| {
            AppError::database(format!("Unknown App Attest environment: {environment}"))
        })?,
        sign_count: u32::try_from(sign_count)
            .map_err(|e| AppError::database(format!("sign_count out of range: {e}")))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| AppError::database(format!("Failed to get created_at: {e}")))?,
        last_used_at: row
            .try_get("last_used_at")
            .map_err(|e| AppError::database(format!("Failed to get last_used_at: {e}")))?,
    })
}

/// Emit the whole [`AppAttestKeyRepository`] implementation for one backend
/// type. The body names its consts and helpers unqualified, so the invoking
/// shell must `use` every one of them.
macro_rules! impl_app_attest_key_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl AppAttestKeyRepository for $ty {
            async fn register_key(&self, key: &AppAttestKey) -> AppResult<bool> {
                let result = sqlx::query(REGISTER_KEY_SQL)
                    .bind(&key.key_id)
                    .bind(&key.public_key)
                    .bind(key.environment.as_str())
                    .bind(i64::from(key.sign_count))
                    .bind(key.created_at)
                    .bind(key.last_used_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to register App Attest key: {e}"))
                    })?;
                Ok(result.rows_affected() == 1)
            }

            async fn find_key(&self, key_id: &str) -> AppResult<Option<AppAttestKey>> {
                let row = sqlx::query(FIND_KEY_SQL)
                    .bind(key_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read App Attest key: {e}"))
                    })?;
                row.map(|row| key_from_row(&row)).transpose()
            }

            async fn advance_counter(
                &self,
                key_id: &str,
                sign_count: u32,
                now: DateTime<Utc>,
            ) -> AppResult<bool> {
                let result = sqlx::query(ADVANCE_COUNTER_SQL)
                    .bind(key_id)
                    .bind(i64::from(sign_count))
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to advance App Attest key counter: {e}"))
                    })?;
                Ok(result.rows_affected() == 1)
            }
        }
    };
}
pub(crate) use impl_app_attest_key_repository;
