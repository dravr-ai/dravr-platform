// ABOUTME: Shared statements and body for first-party refresh tokens — store, exchange once, revoke a family or a user
// ABOUTME: One SQL text per operation; each backend shell supplies only the uuid casts its user_id column needs

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Session refresh tokens, written once.
//!
//! Only the token's HMAC is stored, computed through the same blind-index key
//! the OAuth2 server's refresh tokens use, so a database read cannot rebuild
//! a usable credential. Every use rotates the token and revokes the one it
//! replaced; `family_id` ties the chain together so a replayed token kills
//! the whole chain.
//!
//! The two backends differ in one respect only: `user_id` is a `uuid` column
//! on Postgres and `TEXT` on `SQLite`. The id is always bound as text, so
//! Postgres casts the parameter `::uuid` on the way in and the column `::text`
//! on the way out, where `SQLite` needs neither. Those two suffixes are the
//! macro's arguments; the statements, the binds and the row decode exist once.
//!
//! `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
//! Postgres, so one statement serves both drivers and cannot drift between
//! them. Timestamps bind as [`DateTime<Utc>`] on both: `TIMESTAMPTZ` on
//! Postgres, RFC 3339 text on `SQLite`, which orders correctly for the
//! `expires_at > now` filter because every value shares one offset and width.

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::SessionRefreshToken;
use uuid::Uuid;

/// Store a freshly issued token under its family. `$uuid` is the cast the
/// text-bound user id needs to land in that backend's `user_id` column.
macro_rules! store_token_sql {
    ($uuid:literal) => {
        concat!(
            "
            INSERT INTO session_refresh_tokens
                (token_hash, family_id, user_id, tenant_id, created_at, expires_at)
            VALUES ($1, $2, $3",
            $uuid,
            ", $4, $5, $6)
            "
        )
    };
}
pub(crate) use store_token_sql;

/// Check-and-revoke in one statement: a second exchange of the same token,
/// even a concurrent one, matches zero rows. `$text` is the cast that reads
/// `user_id` back as text on that backend.
macro_rules! consume_token_sql {
    ($text:literal) => {
        concat!(
            "
            UPDATE session_refresh_tokens
            SET revoked_at = $2
            WHERE token_hash = $1
              AND revoked_at IS NULL
              AND expires_at > $2
            RETURNING family_id, user_id",
            $text,
            " AS user_id, tenant_id, created_at, expires_at
            "
        )
    };
}
pub(crate) use consume_token_sql;

/// Revoke every live member of the family the token belongs to.
pub(crate) const REVOKE_TOKEN_FAMILY_SQL: &str = r"
            UPDATE session_refresh_tokens
            SET revoked_at = $2
            WHERE revoked_at IS NULL
              AND family_id = (
                  SELECT family_id FROM session_refresh_tokens WHERE token_hash = $1
              )
            ";

/// Revoke every live token the user holds. `$uuid` as in [`store_token_sql`].
macro_rules! revoke_user_tokens_sql {
    ($uuid:literal) => {
        concat!(
            "
            UPDATE session_refresh_tokens
            SET revoked_at = $2
            WHERE user_id = $1",
            $uuid,
            " AND revoked_at IS NULL
            "
        )
    };
}
pub(crate) use revoke_user_tokens_sql;

/// Decode the record a consumed token returned. `user_id` arrives as text on
/// both backends (see [`consume_token_sql`]) and is parsed here, so one
/// extractor serves both drivers. `try_get` throughout, never `Row::get`, so a
/// corrupt row surfaces as a recoverable error rather than a panic.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded,
/// or when `user_id` is not a uuid.
pub(crate) fn token_from_row<R>(row: &R) -> AppResult<SessionRefreshToken>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let user_id: String = row
        .try_get("user_id")
        .map_err(|e| AppError::database(format!("Failed to get user_id: {e}")))?;
    Ok(SessionRefreshToken {
        family_id: row
            .try_get("family_id")
            .map_err(|e| AppError::database(format!("Failed to get family_id: {e}")))?,
        user_id: Uuid::parse_str(&user_id)
            .map_err(|e| AppError::database(format!("Failed to parse user_id: {e}")))?,
        tenant_id: row
            .try_get("tenant_id")
            .map_err(|e| AppError::database(format!("Failed to get tenant_id: {e}")))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| AppError::database(format!("Failed to get created_at: {e}")))?,
        expires_at: row
            .try_get("expires_at")
            .map_err(|e| AppError::database(format!("Failed to get expires_at: {e}")))?,
    })
}

/// Emit the whole [`SessionRefreshTokenRepository`] implementation for one
/// backend type.
///
/// `$uuid` is the cast a text-bound user id needs to enter that backend's
/// `user_id` column (`"::uuid"` on Postgres, `""` on `SQLite`); `$text` is the
/// cast that reads it back as text (`"::text"` on Postgres, `""` on `SQLite`).
///
/// The body is written once here; each backend's shell invokes it with its
/// own type, and sqlx resolves the driver from `self.pool()` per expansion.
/// The body names its consts, helpers and types unqualified, so the invoking
/// shell must `use` every one of them.
macro_rules! impl_session_refresh_token_repository {
    ($ty:ty, $uuid:literal, $text:literal) => {
        #[async_trait::async_trait]
        impl SessionRefreshTokenRepository for $ty {
            async fn store_token(
                &self,
                token: &str,
                record: &SessionRefreshToken,
            ) -> AppResult<()> {
                let token_hash = HasEncryption::hash_token_for_storage(self, token)?;

                sqlx::query(store_token_sql!($uuid))
                    .bind(&token_hash)
                    .bind(&record.family_id)
                    .bind(record.user_id.to_string())
                    .bind(&record.tenant_id)
                    .bind(record.created_at)
                    .bind(record.expires_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store session refresh token: {e}"))
                    })?;

                Ok(())
            }

            async fn consume_token(
                &self,
                token: &str,
                now: DateTime<Utc>,
            ) -> AppResult<Option<SessionRefreshToken>> {
                let token_hash = HasEncryption::hash_token_for_storage(self, token)?;

                let row = sqlx::query(consume_token_sql!($text))
                    .bind(&token_hash)
                    .bind(now)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to consume session refresh token: {e}"))
                    })?;

                row.map(|row| token_from_row(&row)).transpose()
            }

            async fn revoke_token_family(&self, token: &str, now: DateTime<Utc>) -> AppResult<u64> {
                let token_hash = HasEncryption::hash_token_for_storage(self, token)?;

                let result = sqlx::query(REVOKE_TOKEN_FAMILY_SQL)
                    .bind(&token_hash)
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to revoke session refresh token family: {e}"
                        ))
                    })?;

                Ok(result.rows_affected())
            }

            async fn revoke_user_tokens(
                &self,
                user_id: Uuid,
                now: DateTime<Utc>,
            ) -> AppResult<u64> {
                let result = sqlx::query(revoke_user_tokens_sql!($uuid))
                    .bind(user_id.to_string())
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to revoke user session refresh tokens: {e}"
                        ))
                    })?;

                Ok(result.rows_affected())
            }
        }
    };
}
pub(crate) use impl_session_refresh_token_repository;
