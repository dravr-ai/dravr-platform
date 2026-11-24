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
//! and every timestamp as [`Timestamp`] — including the "now" an expiry
//! is compared against, bound rather than taken from `CURRENT_TIMESTAMP`, so
//! that on `SQLite` the comparison is between two RFC 3339 texts of one
//! width and a key that expired earlier today is seen as expired.
//!
//! The rate-limit columns store the model's value as a 32-bit integer on
//! both backends (`INTEGER NOT NULL` on Postgres), saturating at `i32::MAX`.
//! `rate_limit_requests` is nullable on `SQLite`, where enterprise keys were
//! once stored as NULL to mean unlimited; a NULL reads back as `u32::MAX`.

use async_trait::async_trait;
use dravr_tronc::dates::{CivilDate, Timestamp};
use pierre_core::errors::{AppError, AppResult};

use pierre_core::models::{ApiKey, ApiKeyRequestDay, ApiKeyTier, AGENT_TURN_ROUTE};
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
    /// Deactivate an API key, ending in the same transaction the support
    /// sessions an admin started with it (carnet#1511)
    async fn deactivate(&self, api_key_id: &str, user_id: Uuid) -> AppResult<()>;
    /// Get API key by ID, optionally scoped to a specific user for ownership enforcement
    async fn get_by_id(&self, api_key_id: &str, user_id: Option<Uuid>)
        -> AppResult<Option<ApiKey>>;
    /// Deactivate every active key whose expiry has passed, returning how many.
    async fn cleanup_expired(&self) -> AppResult<u64>;
    /// The REST requests `user_id`'s keys made at or after `since` that
    /// answered with a 2xx, counted per UTC day and key, newest day first
    /// (the athlete's "who read my data" feed, carnet#1543).
    ///
    /// Read from `api_key_usage`, the row the request-budget layer writes for
    /// every request a key admits. Two kinds of request are left out, each
    /// already one row of the feed: a key's calls through `/mcp` (each tool
    /// call is the athlete's own `llm_usage` row, which names the tool, whose
    /// data it read and whether it answered), and a chat message
    /// ([`AGENT_TURN_ROUTE`]), which is its agent turn's row.
    async fn list_request_days(
        &self,
        user_id: Uuid,
        since: Timestamp,
    ) -> AppResult<Vec<ApiKeyRequestDay>>;
}

/// The `api_key_usage.endpoint` shape of a REST request: the request-budget
/// layer names it `METHOD /route`, while a request through `/mcp` names the
/// tool or JSON-RPC method it called (`report_request_operation`).
pub(crate) const REST_ENDPOINT_PATTERN: &str = "% /%";

/// The `/mcp` endpoint itself and everything under it (`GET /mcp/tools`),
/// named as routes when a request reported no operation: not REST data reads.
pub(crate) const MCP_ENDPOINT_PATTERNS: [&str; 2] = ["% /mcp", "% /mcp/%"];

/// The `api_key_usage.endpoint` of a chat message, which starts an agent turn.
pub(crate) fn agent_turn_endpoint() -> String {
    format!("POST {AGENT_TURN_ROUTE}")
}

/// The lowest and highest status of a request that answered with a result.
pub(crate) const SUCCESS_STATUS_BAND: (i16, i16) = (200, 299);

/// The REST requests of one user's keys since `$2` with a status in
/// `[$3, $4]`, per UTC day and key, newest day first. `$day` is the engine's
/// spelling of a row's UTC day as `YYYY-MM-DD` text, as in
/// `llm_usage_daily_series_sql!`. `$5` is [`REST_ENDPOINT_PATTERN`], `$6` and
/// `$7` the [`MCP_ENDPOINT_PATTERNS`], `$8` [`agent_turn_endpoint`].
macro_rules! api_key_request_days_sql {
    ($day:literal) => {
        concat!(
            "SELECT ",
            $day,
            " AS day, k.id AS key_id, k.name AS key_name, k.key_prefix AS key_prefix, \
                    CAST(COUNT(*) AS BIGINT) AS requests \
             FROM api_key_usage u JOIN api_keys k ON u.api_key_id = k.id \
             WHERE k.user_id = $1 AND u.timestamp >= $2 \
               AND u.status_code >= $3 AND u.status_code <= $4 \
               AND u.endpoint LIKE $5 AND u.endpoint NOT LIKE $6 AND u.endpoint NOT LIKE $7 \
               AND u.endpoint <> $8 \
             GROUP BY ",
            $day,
            ", k.id, k.name, k.key_prefix \
             ORDER BY day DESC, k.name, k.id"
        )
    };
}
pub(crate) use api_key_request_days_sql;

/// Decode one row of [`api_key_request_days_sql!`].
///
/// # Errors
/// Returns a database error naming the column that cannot be decoded, or the
/// day that is not a `YYYY-MM-DD` date.
pub(crate) fn request_day_from_row<R>(row: &R) -> AppResult<ApiKeyRequestDay>
where
    R: sqlx::Row,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i64: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
{
    let column = |col: &str, e: sqlx::Error| {
        AppError::database(format!("Failed to get an API key request day's {col}: {e}"))
    };
    let day: String = row.try_get("day").map_err(|e| column("day", e))?;
    let requests: i64 = row.try_get("requests").map_err(|e| column("requests", e))?;
    Ok(ApiKeyRequestDay {
        day: CivilDate::parse_from_str(&day, "%Y-%m-%d").map_err(|e| {
            AppError::database(format!(
                "An API key request day is not a date ({day:?}): {e}"
            ))
        })?,
        key_id: row.try_get("key_id").map_err(|e| column("key_id", e))?,
        key_name: row.try_get("key_name").map_err(|e| column("key_name", e))?,
        key_prefix: row
            .try_get("key_prefix")
            .map_err(|e| column("key_prefix", e))?,
        requests: u32::try_from(requests).unwrap_or(u32::MAX),
    })
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

/// One key by id regardless of owner, for admin callers.
pub(crate) const GET_ANY_API_KEY_BY_ID_SQL: &str = concat!(
    "SELECT ",
    api_key_columns!(""),
    " FROM api_keys WHERE id = $1"
);

/// Deactivate every active key whose expiry lies before `$1`.
pub(crate) const CLEANUP_EXPIRED_API_KEYS_SQL: &str = "UPDATE api_keys SET is_active = false WHERE expires_at IS NOT NULL AND expires_at < $1 AND is_active = true";

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
    String: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<String>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    bool: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    i32: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<i32>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Timestamp: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
    Option<Timestamp>: sqlx::Type<R::Database> + for<'a> sqlx::Decode<'a, R::Database>,
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
    ($ty:ty, $ids:ident, day = $day:literal) => {
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
                let deactivate_error = |e: sqlx::Error| {
                    AppError::database(format!("Failed to deactivate API key: {e}"))
                };
                let mut tx = self.pool().begin().await.map_err(deactivate_error)?;
                sqlx::query(DEACTIVATE_API_KEY_SQL)
                    .bind(api_key_id)
                    .bind($ids::bind(user_id))
                    .execute(&mut *tx)
                    .await
                    .map_err(deactivate_error)?;
                // A support session started with this key ends with it
                // (carnet#1511).
                let key = $crate::repositories::support_binding::RevokedCredential::ApiKey {
                    key_id: api_key_id,
                    user_id,
                };
                Self::end_sessions_of_revoked(&mut tx, key).await?;
                tx.commit().await.map_err(deactivate_error)?;

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

            async fn cleanup_expired(&self) -> AppResult<u64> {
                let result = sqlx::query(CLEANUP_EXPIRED_API_KEYS_SQL)
                    .bind(Timestamp::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to cleanup expired API keys: {e}"))
                    })?;

                Ok(result.rows_affected())
            }

            async fn list_request_days(
                &self,
                user_id: Uuid,
                since: Timestamp,
            ) -> AppResult<Vec<ApiKeyRequestDay>> {
                let (lowest, highest) = SUCCESS_STATUS_BAND;
                let [mcp, under_mcp] = MCP_ENDPOINT_PATTERNS;
                sqlx::query(api_key_request_days_sql!($day))
                    .bind($ids::bind(user_id))
                    .bind(since)
                    .bind(lowest)
                    .bind(highest)
                    .bind(REST_ENDPOINT_PATTERN)
                    .bind(mcp)
                    .bind(under_mcp)
                    .bind(agent_turn_endpoint())
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list API key request days: {e}"))
                    })?
                    .iter()
                    .map(request_day_from_row)
                    .collect()
            }
        }
    };
}
pub(crate) use impl_api_key_repository;
