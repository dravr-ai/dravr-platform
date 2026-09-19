// ABOUTME: Repository trait, statements and shared body for user MCP tokens (AI client authentication)
// ABOUTME: Mint, validate, list, read, revoke and sweep, every one scoped by the owning user; written once per backend by macro
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Display;

use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    CreateUserMcpTokenRequest, UserMcpToken, UserMcpTokenCreated, UserMcpTokenInfo,
};
use rand::RngCore;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// User MCP token management repository.
///
/// A permission surface: the raw token is the credential an AI client
/// presents on the MCP transport. Every read, listing and revocation is
/// scoped by the owning `user_id`, and the per-token reads take
/// `id AND user_id` so a token id on its own reaches nothing.
#[async_trait]
pub trait UserMcpTokenRepository: Send + Sync {
    /// Create a new user MCP token for AI client authentication
    async fn create_token(
        &self,
        user_id: Uuid,
        request: &CreateUserMcpTokenRequest,
    ) -> AppResult<UserMcpTokenCreated>;
    /// Validate a user MCP token and return the associated user ID
    async fn validate_token(&self, token_value: &str) -> AppResult<Uuid>;
    /// List all MCP tokens for a user
    async fn list_tokens(&self, user_id: Uuid) -> AppResult<Vec<UserMcpTokenInfo>>;
    /// Revoke a user MCP token
    async fn revoke_token(&self, token_id: &str, user_id: Uuid) -> AppResult<()>;
    /// Get a user MCP token by ID
    async fn get_token(&self, token_id: &str, user_id: Uuid) -> AppResult<Option<UserMcpToken>>;
    /// Cleanup expired user MCP tokens (mark as revoked)
    async fn cleanup_expired_tokens(&self) -> AppResult<u64>;
}

/// How many leading characters of the raw token are stored in the clear
/// as its lookup key; the rest is only ever compared through its hash.
const TOKEN_PREFIX_LEN: usize = 12;

/// Mint a raw token: 32 random bytes, url-safe base64, behind a `pmcp_` tag.
pub(crate) fn generate_mcp_token() -> String {
    let mut rng = rand::rng();
    let mut bytes = [0u8; 32];
    rng.fill_bytes(&mut bytes);
    format!("pmcp_{}", URL_SAFE_NO_PAD.encode(bytes))
}

/// The stored form of a raw token: its hex SHA-256.
pub(crate) fn hash_mcp_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// The clear-text lookup key of a raw token.
pub(crate) fn mcp_token_prefix(token: &str) -> String {
    token.chars().take(TOKEN_PREFIX_LEN).collect()
}

/// Mint a row. `last_used_at` starts NULL, `usage_count` at 0, and the token
/// is live; `FALSE` is the boolean spelling both engines accept.
///
/// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
/// Postgres, so one statement serves both backends and cannot drift between
/// them. `user_id` binds through the backend's uuid codec (see
/// [`super::uuid_columns`]); every other bind is a plain `&str`,
/// `Option<DateTime<Utc>>` or `DateTime<Utc>` both drivers encode alike.
pub(crate) const CREATE_MCP_TOKEN_SQL: &str = r"
            INSERT INTO user_mcp_tokens (
                id, user_id, name, token_hash, token_prefix,
                expires_at, last_used_at, usage_count, is_revoked, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, NULL, 0, FALSE, $7)
            ";

/// The row a raw token resolves to, by its clear prefix and its hash.
pub(crate) const FIND_MCP_TOKEN_BY_VALUE_SQL: &str = r"
            SELECT id, user_id, expires_at, is_revoked
            FROM user_mcp_tokens
            WHERE token_prefix = $1 AND token_hash = $2
            ";

/// Count one more use.
pub(crate) const TOUCH_MCP_TOKEN_SQL: &str = r"
            UPDATE user_mcp_tokens
            SET last_used_at = $1, usage_count = usage_count + 1
            WHERE id = $2
            ";

/// A user's tokens, newest first, without the hash.
pub(crate) const LIST_MCP_TOKENS_SQL: &str = r"
            SELECT id, name, token_prefix, expires_at, last_used_at,
                   usage_count, is_revoked, created_at
            FROM user_mcp_tokens
            WHERE user_id = $1
            ORDER BY created_at DESC
            ";

/// Revoke one token, only if the caller owns it.
pub(crate) const REVOKE_MCP_TOKEN_SQL: &str = r"
            UPDATE user_mcp_tokens
            SET is_revoked = TRUE
            WHERE id = $1 AND user_id = $2
            ";

/// One token, only if the caller owns it.
pub(crate) const GET_MCP_TOKEN_SQL: &str = r"
            SELECT id, user_id, name, token_hash, token_prefix,
                   expires_at, last_used_at, usage_count, is_revoked, created_at
            FROM user_mcp_tokens
            WHERE id = $1 AND user_id = $2
            ";

/// Revoke every live token whose window has closed.
pub(crate) const SWEEP_EXPIRED_MCP_TOKENS_SQL: &str = r"
            UPDATE user_mcp_tokens
            SET is_revoked = TRUE
            WHERE expires_at IS NOT NULL
            AND expires_at < $1
            AND is_revoked = FALSE
            ";

/// The error for a column of this table that would not decode.
pub(crate) fn mcp_token_column_error(name: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to get column '{name}': {e}"))
}

/// The `usage_count` column as the domain sees it: `INTEGER` on both
/// engines, never negative by construction.
///
/// # Errors
/// Returns an internal error if a negative count is ever stored.
pub(crate) fn usage_count_from_column(raw: i32) -> AppResult<u32> {
    u32::try_from(raw)
        .map_err(|e| AppError::internal(format!("Integer conversion failed for usage_count: {e}")))
}

/// Emit the whole [`UserMcpTokenRepository`] implementation for one backend
/// type. The body is written once here; each backend's shell invokes it with
/// its own type, its driver's row type and its uuid codec, and sqlx resolves
/// the driver from `self.pool()` per expansion.
///
/// `$row` is the driver's row type and `$ids` the codec in
/// [`super::uuid_columns`] for how that backend's `user_id` column binds and
/// reads; the row parsers are emitted inside the macro because that one read
/// is the only per-driver decode.
macro_rules! impl_user_mcp_token_repository {
    ($ty:ty, $row:ty, $ids:ident) => {
        /// Decode one full token row via `try_get` only, so a corrupt row
        /// surfaces as a recoverable error rather than a panic.
        fn mcp_token_from_row(row: &$row) -> AppResult<UserMcpToken> {
            Ok(UserMcpToken {
                id: row
                    .try_get("id")
                    .map_err(|e| mcp_token_column_error("id", e))?,
                user_id: $ids::read(row, "user_id")?,
                name: row
                    .try_get("name")
                    .map_err(|e| mcp_token_column_error("name", e))?,
                token_hash: row
                    .try_get("token_hash")
                    .map_err(|e| mcp_token_column_error("token_hash", e))?,
                token_prefix: row
                    .try_get("token_prefix")
                    .map_err(|e| mcp_token_column_error("token_prefix", e))?,
                expires_at: row
                    .try_get("expires_at")
                    .map_err(|e| mcp_token_column_error("expires_at", e))?,
                last_used_at: row
                    .try_get("last_used_at")
                    .map_err(|e| mcp_token_column_error("last_used_at", e))?,
                usage_count: usage_count_from_column(
                    row.try_get("usage_count")
                        .map_err(|e| mcp_token_column_error("usage_count", e))?,
                )?,
                is_revoked: row
                    .try_get("is_revoked")
                    .map_err(|e| mcp_token_column_error("is_revoked", e))?,
                created_at: row
                    .try_get("created_at")
                    .map_err(|e| mcp_token_column_error("created_at", e))?,
            })
        }

        /// Decode one listing row (no owner, no hash).
        fn mcp_token_info_from_row(row: &$row) -> AppResult<UserMcpTokenInfo> {
            Ok(UserMcpTokenInfo {
                id: row
                    .try_get("id")
                    .map_err(|e| mcp_token_column_error("id", e))?,
                name: row
                    .try_get("name")
                    .map_err(|e| mcp_token_column_error("name", e))?,
                token_prefix: row
                    .try_get("token_prefix")
                    .map_err(|e| mcp_token_column_error("token_prefix", e))?,
                expires_at: row
                    .try_get("expires_at")
                    .map_err(|e| mcp_token_column_error("expires_at", e))?,
                last_used_at: row
                    .try_get("last_used_at")
                    .map_err(|e| mcp_token_column_error("last_used_at", e))?,
                usage_count: usage_count_from_column(
                    row.try_get("usage_count")
                        .map_err(|e| mcp_token_column_error("usage_count", e))?,
                )?,
                is_revoked: row
                    .try_get("is_revoked")
                    .map_err(|e| mcp_token_column_error("is_revoked", e))?,
                created_at: row
                    .try_get("created_at")
                    .map_err(|e| mcp_token_column_error("created_at", e))?,
            })
        }

        #[async_trait::async_trait]
        impl UserMcpTokenRepository for $ty {
            async fn create_token(
                &self,
                user_id: Uuid,
                request: &CreateUserMcpTokenRequest,
            ) -> AppResult<UserMcpTokenCreated> {
                let token_value = generate_mcp_token();
                let token_hash = hash_mcp_token(&token_value);
                let token_prefix = mcp_token_prefix(&token_value);
                let id = Uuid::new_v4().to_string();
                let now = Utc::now();

                let expires_at = request
                    .expires_in_days
                    .map(|days| now + Duration::days(i64::from(days)));

                sqlx::query(CREATE_MCP_TOKEN_SQL)
                    .bind(&id)
                    .bind($ids::bind(user_id))
                    .bind(&request.name)
                    .bind(&token_hash)
                    .bind(&token_prefix)
                    .bind(expires_at)
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to create user MCP token: {e}"))
                    })?;

                let token = UserMcpToken {
                    id,
                    user_id,
                    name: request.name.clone(),
                    token_hash,
                    token_prefix,
                    expires_at,
                    last_used_at: None,
                    usage_count: 0,
                    is_revoked: false,
                    created_at: now,
                };

                Ok(UserMcpTokenCreated { token, token_value })
            }

            async fn validate_token(&self, token_value: &str) -> AppResult<Uuid> {
                let row = sqlx::query(FIND_MCP_TOKEN_BY_VALUE_SQL)
                    .bind(mcp_token_prefix(token_value))
                    .bind(hash_mcp_token(token_value))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to validate user MCP token: {e}"))
                    })?;

                let row = row.ok_or_else(|| AppError::auth_invalid("Invalid MCP token"))?;

                let is_revoked: bool = row
                    .try_get("is_revoked")
                    .map_err(|e| mcp_token_column_error("is_revoked", e))?;
                if is_revoked {
                    return Err(AppError::auth_invalid("MCP token has been revoked"));
                }

                let expires_at: Option<DateTime<Utc>> = row
                    .try_get("expires_at")
                    .map_err(|e| mcp_token_column_error("expires_at", e))?;
                if expires_at.is_some_and(|exp| exp < Utc::now()) {
                    return Err(AppError::auth_invalid("MCP token has expired"));
                }

                let token_id: String = row
                    .try_get("id")
                    .map_err(|e| mcp_token_column_error("id", e))?;
                sqlx::query(TOUCH_MCP_TOKEN_SQL)
                    .bind(Utc::now())
                    .bind(&token_id)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update user MCP token usage: {e}"))
                    })?;

                $ids::read(&row, "user_id")
            }

            async fn list_tokens(&self, user_id: Uuid) -> AppResult<Vec<UserMcpTokenInfo>> {
                let rows = sqlx::query(LIST_MCP_TOKENS_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to list user MCP tokens: {e}"))
                    })?;
                rows.iter().map(mcp_token_info_from_row).collect()
            }

            async fn revoke_token(&self, token_id: &str, user_id: Uuid) -> AppResult<()> {
                let result = sqlx::query(REVOKE_MCP_TOKEN_SQL)
                    .bind(token_id)
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to revoke user MCP token: {e}"))
                    })?;

                if result.rows_affected() == 0 {
                    return Err(AppError::not_found("MCP token not found or unauthorized"));
                }

                Ok(())
            }

            async fn get_token(
                &self,
                token_id: &str,
                user_id: Uuid,
            ) -> AppResult<Option<UserMcpToken>> {
                let row = sqlx::query(GET_MCP_TOKEN_SQL)
                    .bind(token_id)
                    .bind($ids::bind(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get user MCP token: {e}"))
                    })?;

                row.as_ref().map(mcp_token_from_row).transpose()
            }

            async fn cleanup_expired_tokens(&self) -> AppResult<u64> {
                let result = sqlx::query(SWEEP_EXPIRED_MCP_TOKENS_SQL)
                    .bind(Utc::now())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to cleanup expired user MCP tokens: {e}"
                        ))
                    })?;

                Ok(result.rows_affected())
            }
        }
    };
}
pub(crate) use impl_user_mcp_token_repository;
