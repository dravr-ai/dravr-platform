// ABOUTME: Repository trait, statements and shared body for API keys — issue, look up by prefix, list, expire
// ABOUTME: One SQL text per operation; each backend shell supplies only the uuid codec its user_id column needs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! API keys, written once.
//!
//! A key row holds the prefix the caller presents, the SHA-256 of the whole
//! key, its tier and rate limit, and its lifecycle timestamps. `user_id` is
//! a `uuid` column on Postgres and `TEXT` on `SQLite`, so the shell hands the
//! body its [`uuid_columns`](super::uuid_columns) codec. Everything else binds
//! the same way on both drivers: `is_active` as `bool` (`BOOLEAN` on
//! Postgres, `INTEGER` 0/1 on `SQLite`), the two rate-limit columns as `i32`,
//! and every timestamp as [`DateTime<Utc>`] — including the "now" an expiry
//! is compared against, bound rather than taken from `CURRENT_TIMESTAMP`, so
//! that on `SQLite` the comparison is between two RFC 3339 texts of one
//! width and a key that expired earlier today is seen as expired.
//!
//! The rate-limit columns store the model's value as a 32-bit integer on
//! both backends (`INTEGER NOT NULL` on Postgres), saturating at `i32::MAX`.
//! `rate_limit_requests` is nullable on `SQLite`, where enterprise keys were
//! once stored as NULL to mean unlimited; a NULL reads back as `u32::MAX`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use std::fmt::Write;

use pierre_core::models::{ApiKey, ApiKeyTier};
use uuid::Uuid;

/// API key management repository
#[async_trait]
pub trait ApiKeyRepository: Send + Sync {
    /// Create a new API key
    async fn create(&self, api_key: &ApiKey) -> AppResult<()>;
    /// Get API key by its prefix and hash
    async fn get_by_prefix(&self, prefix: &str, hash: &str) -> AppResult<Option<ApiKey>>;
    /// Get all API keys for a user
    async fn get_for_user(&self, user_id: Uuid) -> AppResult<Vec<ApiKey>>;
    /// Update API key last used timestamp
    async fn update_last_used(&self, api_key_id: &str) -> AppResult<()>;
    /// Deactivate an API key
    async fn deactivate(&self, api_key_id: &str, user_id: Uuid) -> AppResult<()>;
    /// Get API key by ID, optionally scoped to a specific user for ownership enforcement
    async fn get_by_id(&self, api_key_id: &str, user_id: Option<Uuid>)
        -> AppResult<Option<ApiKey>>;
    /// Get API keys with optional filters
    async fn get_filtered(
        &self,
        user_email: Option<&str>,
        active_only: bool,
        limit: Option<i32>,
        offset: Option<i32>,
    ) -> AppResult<Vec<ApiKey>>;
    /// Deactivate every active key whose expiry has passed, returning how many.
    async fn cleanup_expired(&self) -> AppResult<u64>;
}

/// The columns the [`ApiKey`] model carries, in the order every read lists them.
macro_rules! api_key_columns {
    ($prefix:literal) => {
        concat!(
            $prefix,
            "id, ",
            $prefix,
            "user_id, ",
            $prefix,
            "name, ",
            $prefix,
            "description, ",
            $prefix,
            "key_hash, ",
            $prefix,
            "key_prefix, ",
            $prefix,
            "tier, ",
            $prefix,
            "rate_limit_requests, ",
            $prefix,
            "rate_limit_window_seconds, ",
            $prefix,
            "is_active, ",
            $prefix,
            "expires_at, ",
            $prefix,
            "last_used_at, ",
            $prefix,
            "created_at"
        )
    };
}

/// Store a freshly issued key.
pub(crate) const CREATE_API_KEY_SQL: &str = concat!(
    "INSERT INTO api_keys (",
    api_key_columns!(""),
    ") VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)"
);

/// The one lookup every API-key authentication depends on: the active key
/// behind the presented prefix whose hash matches. Matches `key_prefix`, not
/// `id` — the Postgres copy once read `WHERE id LIKE $1` and rejected every
/// key in deployed environments while `SQLite` kept working.
pub(crate) const GET_API_KEY_BY_PREFIX_SQL: &str = concat!(
    "SELECT ",
    api_key_columns!(""),
    " FROM api_keys WHERE key_prefix = $1 AND key_hash = $2 AND is_active = true"
);

/// Every key the user owns, newest first.
pub(crate) const GET_USER_API_KEYS_SQL: &str = concat!(
    "SELECT ",
    api_key_columns!(""),
    " FROM api_keys WHERE user_id = $1 ORDER BY created_at DESC"
);

/// Stamp the moment a key was last presented.
pub(crate) const UPDATE_API_KEY_LAST_USED_SQL: &str =
    "UPDATE api_keys SET last_used_at = CURRENT_TIMESTAMP WHERE id = $1";

/// Deactivate one of the user's keys; a key that is not theirs, or does not
/// exist, is left alone without error.
pub(crate) const DEACTIVATE_API_KEY_SQL: &str =
    "UPDATE api_keys SET is_active = false WHERE id = $1 AND user_id = $2";

/// One key by id, only when the user owns it.
pub(crate) const GET_USER_API_KEY_BY_ID_SQL: &str = concat!(
    "SELECT ",
    api_key_columns!(""),
    " FROM api_keys WHERE id = $1 AND user_id = $2"
);

/// One key by id regardless of owner, for operator callers.
pub(crate) const GET_ANY_API_KEY_BY_ID_SQL: &str = concat!(
    "SELECT ",
    api_key_columns!(""),
    " FROM api_keys WHERE id = $1"
);

/// Deactivate every active key whose expiry lies before `$1`.
pub(crate) const CLEANUP_EXPIRED_API_KEYS_SQL: &str = "UPDATE api_keys SET is_active = false WHERE expires_at IS NOT NULL AND expires_at < $1 AND is_active = true";

/// The operator listing: every key, optionally only one user's (by email)
/// and only the active ones, newest first, with an optional row window. The
/// email, limit and offset bind in that order as `$1..`, each only when
/// given; `offset` needs `limit`.
///
/// # Errors
/// Returns a database error when a clause cannot be appended.
pub(crate) fn filtered_api_keys_sql(
    user_email: Option<&str>,
    active_only: bool,
    limit: Option<i32>,
    offset: Option<i32>,
) -> AppResult<String> {
    let mut sql = concat!("SELECT ", api_key_columns!("ak."), " FROM api_keys ak").to_owned();
    let mut conditions = Vec::new();
    let mut param_count = 0;

    if user_email.is_some() {
        sql.push_str(" JOIN users u ON ak.user_id = u.id");
        param_count += 1;
        conditions.push(format!("u.email = ${param_count}"));
    }
    if active_only {
        conditions.push("ak.is_active = true".to_owned());
    }
    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }
    sql.push_str(" ORDER BY ak.created_at DESC");

    if limit.is_some() {
        param_count += 1;
        write!(sql, " LIMIT ${param_count}")
            .map_err(|e| AppError::database(format!("Failed to write LIMIT clause: {e}")))?;
        if offset.is_some() {
            param_count += 1;
            write!(sql, " OFFSET ${param_count}")
                .map_err(|e| AppError::database(format!("Failed to write OFFSET clause: {e}")))?;
        }
    }
    Ok(sql)
}

/// The value a rate-limit column stores for the model's `u32`: the column is
/// a 32-bit integer on both backends, so an unlimited (`u32::MAX`) key holds
/// `i32::MAX`.
pub(crate) fn rate_limit_column(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// Decode one `api_keys` row into the model. `user_id` is read by the
/// caller through its backend's codec, since the column is native uuid on
/// Postgres and text on `SQLite`; every other column decodes the same way on
/// both drivers. `try_get` throughout, never `Row::get`, so a corrupt row
/// surfaces as a recoverable error rather than a panic.
///
/// # Errors
/// Returns a database error naming the first column that cannot be decoded,
/// or an internal error when the stored tier is not one the model knows.
pub(crate) fn api_key_from_row<R>(row: &R, user_id: Uuid) -> AppResult<ApiKey>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i32: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<i32>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    DateTime<Utc>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<DateTime<Utc>>: for<'a> sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column = |col: &str, e: sqlx::Error| {
        AppError::database(format!("Failed to get api_keys.{col}: {e}"))
    };
    let tier_str: String = row.try_get("tier").map_err(|e| column("tier", e))?;
    let tier = tier_str
        .parse::<ApiKeyTier>()
        .map_err(|e| AppError::internal(format!("Failed to parse tier: {e}")))?;
    let stored_limit: Option<i32> = row
        .try_get("rate_limit_requests")
        .map_err(|e| column("rate_limit_requests", e))?;
    let rate_limit_requests = stored_limit.map_or(Ok(u32::MAX), |stored| {
        u32::try_from(stored).map_err(|e| {
            AppError::internal(format!(
                "Integer conversion failed for rate_limit_requests: {e}"
            ))
        })
    })?;
    let window: i32 = row
        .try_get("rate_limit_window_seconds")
        .map_err(|e| column("rate_limit_window_seconds", e))?;

    Ok(ApiKey {
        id: row.try_get("id").map_err(|e| column("id", e))?,
        user_id,
        name: row.try_get("name").map_err(|e| column("name", e))?,
        description: row
            .try_get("description")
            .map_err(|e| column("description", e))?,
        key_hash: row.try_get("key_hash").map_err(|e| column("key_hash", e))?,
        key_prefix: row
            .try_get("key_prefix")
            .map_err(|e| column("key_prefix", e))?,
        tier,
        rate_limit_requests,
        rate_limit_window_seconds: u32::try_from(window).map_err(|e| {
            AppError::internal(format!(
                "Integer conversion failed for rate_limit_window_seconds: {e}"
            ))
        })?,
        is_active: row
            .try_get("is_active")
            .map_err(|e| column("is_active", e))?,
        expires_at: row
            .try_get("expires_at")
            .map_err(|e| column("expires_at", e))?,
        last_used_at: row
            .try_get("last_used_at")
            .map_err(|e| column("last_used_at", e))?,
        created_at: row
            .try_get("created_at")
            .map_err(|e| column("created_at", e))?,
    })
}

/// Emit the whole [`ApiKeyRepository`] implementation for one backend type.
///
/// `$ids` is that backend's [`uuid_columns`](super::uuid_columns) codec, which
/// spells how the `user_id` column binds and reads. The body is written once
/// here; each backend's shell invokes it with its own type, and sqlx resolves
/// the driver from `self.pool()` per expansion.
macro_rules! impl_api_key_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl ApiKeyRepository for $ty {
            async fn create(&self, api_key: &ApiKey) -> AppResult<()> {
                sqlx::query(CREATE_API_KEY_SQL)
                    .bind(&api_key.id)
                    .bind($ids::bind(api_key.user_id))
                    .bind(&api_key.name)
                    .bind(&api_key.description)
                    .bind(&api_key.key_hash)
                    .bind(&api_key.key_prefix)
                    .bind(api_key.tier.as_str())
                    .bind(rate_limit_column(api_key.rate_limit_requests))
                    .bind(rate_limit_column(api_key.rate_limit_window_seconds))
                    .bind(api_key.is_active)
                    .bind(api_key.expires_at)
                    .bind(api_key.last_used_at)
                    .bind(api_key.created_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to create API key: {e}")))?;

                Ok(())
            }

            async fn get_by_prefix(&self, prefix: &str, hash: &str) -> AppResult<Option<ApiKey>> {
                let row = sqlx::query(GET_API_KEY_BY_PREFIX_SQL)
                    .bind(prefix)
                    .bind(hash)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get API key by prefix: {e}"))
                    })?;

                row.map(|row| api_key_from_row(&row, $ids::read(&row, "user_id")?))
                    .transpose()
            }

            async fn get_for_user(&self, user_id: Uuid) -> AppResult<Vec<ApiKey>> {
                let rows = sqlx::query(GET_USER_API_KEYS_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get user API keys: {e}")))?;

                rows.iter()
                    .map(|row| api_key_from_row(row, $ids::read(row, "user_id")?))
                    .collect()
            }

            async fn update_last_used(&self, api_key_id: &str) -> AppResult<()> {
                sqlx::query(UPDATE_API_KEY_LAST_USED_SQL)
                    .bind(api_key_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update API key last used: {e}"))
                    })?;

                Ok(())
            }

            async fn deactivate(&self, api_key_id: &str, user_id: Uuid) -> AppResult<()> {
                sqlx::query(DEACTIVATE_API_KEY_SQL)
                    .bind(api_key_id)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to deactivate API key: {e}"))
                    })?;

                Ok(())
            }

            async fn get_by_id(
                &self,
                api_key_id: &str,
                user_id: Option<Uuid>,
            ) -> AppResult<Option<ApiKey>> {
                let row = if let Some(uid) = user_id {
                    sqlx::query(GET_USER_API_KEY_BY_ID_SQL)
                        .bind(api_key_id)
                        .bind($ids::bind(uid))
                        .fetch_optional(self.pool())
                        .await
                } else {
                    sqlx::query(GET_ANY_API_KEY_BY_ID_SQL)
                        .bind(api_key_id)
                        .fetch_optional(self.pool())
                        .await
                }
                .map_err(|e| AppError::database(format!("Failed to get API key by ID: {e}")))?;

                row.map(|row| api_key_from_row(&row, $ids::read(&row, "user_id")?))
                    .transpose()
            }

            async fn get_filtered(
                &self,
                user_email: Option<&str>,
                active_only: bool,
                limit: Option<i32>,
                offset: Option<i32>,
            ) -> AppResult<Vec<ApiKey>> {
                let sql = filtered_api_keys_sql(user_email, active_only, limit, offset)?;
                let mut query = sqlx::query(&sql);
                if let Some(email) = user_email {
                    query = query.bind(email);
                }
                if let Some(limit) = limit {
                    query = query.bind(limit);
                    if let Some(offset) = offset {
                        query = query.bind(offset);
                    }
                }

                let rows = query
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to list API keys: {e}")))?;

                rows.iter()
                    .map(|row| api_key_from_row(row, $ids::read(row, "user_id")?))
                    .collect()
            }

            async fn cleanup_expired(&self) -> AppResult<u64> {
                let result = sqlx::query(CLEANUP_EXPIRED_API_KEYS_SQL)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to cleanup expired API keys: {e}"))
                    })?;

                Ok(result.rows_affected())
            }
        }
    };
}
pub(crate) use impl_api_key_repository;
