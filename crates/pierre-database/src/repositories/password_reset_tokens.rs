// ABOUTME: Shared statements and body for password-reset tokens — issue, consume once under a lockout, invalidate, rate-limit
// ABOUTME: One SQL text per operation; each backend shell supplies only the uuid codec its user_id column needs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Password-reset tokens, written once.
//!
//! The delivered token is `<selector>.<verifier>`: the selector is stored in
//! clear as the lookup key and only the SHA-256 of the verifier is stored, so
//! a database read cannot rebuild a usable token. A wrong verifier costs one
//! attempt without consuming the token, past the attempt cap the token
//! self-invalidates, and a successful claim flips `used_at` exactly once.
//!
//! The two backends differ in one respect: `user_id` is a `uuid` column on
//! Postgres and `TEXT` on `SQLite`, so the shell hands the body its
//! [`uuid_columns`](super::uuid_columns) codec. The token's own `id` is `TEXT`
//! on both. Timestamps bind as [`DateTime<Utc>`] on both: `TIMESTAMPTZ` on
//! Postgres, RFC 3339 text on `SQLite`, which orders correctly for the
//! `expires_at > now` filter because every value shares one offset and width.
//! The consume runs through the one body every link token shares
//! ([`link_token_consume`](super::link_token_consume)), which binds the
//! attempt cap against the `INTEGER` `attempt_count` column on both drivers.

use pierre_core::errors::AppError;

use super::link_token_consume::{link_token_guess_sql, LinkTokenTable, LINK_TOKEN_MAX_ATTEMPTS};

/// Lifetime of an admin-issued password reset token: one hour. The
/// self-service flow passes its own, shorter TTL.
pub(crate) const RESET_TOKEN_TTL_MINUTES: i64 = 60;

/// Issue one token: the selector half in clear, the verifier half hashed.
pub(crate) const STORE_RESET_TOKEN_SQL: &str = r"
            INSERT INTO password_reset_tokens (id, user_id, selector, token_hash, expires_at, created_by, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ";

/// How a password reset token is consumed ([`consume_link_token`]). The
/// selector is the only lookup key: no global `token_hash` scan.
pub(crate) const RESET_LINK_TOKEN: LinkTokenTable = LinkTokenTable {
    guess_sql: link_token_guess_sql!("password_reset_tokens", "used_at IS NULL", "user_id"),
    lock_sql: "UPDATE password_reset_tokens SET used_at = $1 \
               WHERE selector = $2 AND used_at IS NULL AND attempt_count >= $3",
    claim_sql: "UPDATE password_reset_tokens SET used_at = $1 \
                WHERE selector = $2 AND used_at IS NULL",
    invalid: invalid_reset_token,
    what: "reset token",
    max_attempts: LINK_TOKEN_MAX_ATTEMPTS,
};

/// Spend every live token a user holds, after a successful password change.
pub(crate) const INVALIDATE_USER_RESET_TOKENS_SQL: &str = r"
            UPDATE password_reset_tokens
            SET used_at = $1
            WHERE user_id = $2
              AND used_at IS NULL
            ";

/// Tokens issued for a user since a moment, used or not, for rate limiting.
pub(crate) const COUNT_RECENT_RESET_TOKENS_SQL: &str = r"
            SELECT COUNT(*) AS cnt
            FROM password_reset_tokens
            WHERE user_id = $1
              AND created_at >= $2
            ";

/// One uniform error for every failure mode (unknown selector, expired, wrong
/// verifier, locked out) so the endpoint never reveals which hit.
pub(crate) fn invalid_reset_token() -> AppError {
    AppError::not_found("Password reset token is invalid, expired, or already used")
}

/// Emit the whole [`PasswordResetRepository`](super::PasswordResetRepository)
/// implementation for one backend type.
///
/// `$ids` is that backend's [`uuid_columns`](super::uuid_columns) codec, which
/// spells how the `user_id` column binds and reads. The body is written once
/// here; each backend's shell invokes it with its own type, and sqlx resolves
/// the driver from `self.pool()` per expansion. The body names its consts,
/// helpers and types unqualified, so the invoking shell must `use` every one
/// of them.
macro_rules! impl_password_reset_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl PasswordResetRepository for $ty {
            async fn store_token(
                &self,
                user_id: Uuid,
                selector: &str,
                verifier_hash: &str,
                created_by: &str,
            ) -> AppResult<Uuid> {
                PasswordResetRepository::store_token_with_ttl(
                    self,
                    user_id,
                    selector,
                    verifier_hash,
                    created_by,
                    RESET_TOKEN_TTL_MINUTES,
                )
                .await
            }

            async fn store_token_with_ttl(
                &self,
                user_id: Uuid,
                selector: &str,
                verifier_hash: &str,
                created_by: &str,
                ttl_minutes: i64,
            ) -> AppResult<Uuid> {
                let id = Uuid::new_v4();
                let now = Utc::now();
                let expires_at = now + chrono::Duration::minutes(ttl_minutes);

                sqlx::query(STORE_RESET_TOKEN_SQL)
                    .bind(id.to_string())
                    .bind($ids::bind(user_id))
                    .bind(selector)
                    .bind(verifier_hash)
                    .bind(expires_at)
                    .bind(created_by)
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store password reset token: {e}"))
                    })?;

                Ok(id)
            }

            async fn consume_token(&self, selector: &str, verifier_hash: &str) -> AppResult<Uuid> {
                let row =
                    consume_link_token!(self.pool(), &RESET_LINK_TOKEN, selector, verifier_hash)?;
                $ids::read(&row, "user_id")
            }

            async fn invalidate_tokens(&self, user_id: Uuid) -> AppResult<()> {
                sqlx::query(INVALIDATE_USER_RESET_TOKENS_SQL)
                    .bind(Utc::now())
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to invalidate reset tokens: {e}"))
                    })?;

                Ok(())
            }

            async fn count_recent_tokens(
                &self,
                user_id: Uuid,
                since: DateTime<Utc>,
            ) -> AppResult<i64> {
                let row = sqlx::query(COUNT_RECENT_RESET_TOKENS_SQL)
                    .bind($ids::bind(user_id))
                    .bind(since)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to count recent reset tokens: {e}"))
                    })?;

                row.try_get::<i64, _>("cnt")
                    .map_err(|e| AppError::database(format!("Failed to get cnt: {e}")))
            }
        }
    };
}
pub(crate) use impl_password_reset_repository;
