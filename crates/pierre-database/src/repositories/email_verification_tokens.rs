// ABOUTME: Shared statements and body for email-verification tokens — issue, consume once, rate-limit, stamp verified
// ABOUTME: One SQL text per operation; each backend shell supplies only the uuid codec its user_id column needs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Email-verification tokens, written once.
//!
//! Mirrors the password-reset flow's `<selector>.<verifier>` mechanism against
//! a separate token space: only the SHA-256 of the verifier is stored, a wrong
//! guess costs one attempt without consuming the token, and past the attempt
//! cap the token self-invalidates.
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

/// Issue one token: the selector half in clear, the verifier half hashed.
pub(crate) const STORE_VERIFICATION_TOKEN_SQL: &str = r"
            INSERT INTO email_verification_tokens (id, user_id, selector, token_hash, expires_at, created_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ";

/// How a verification token is consumed ([`consume_link_token`]).
pub(crate) const VERIFICATION_LINK_TOKEN: LinkTokenTable = LinkTokenTable {
    guess_sql: link_token_guess_sql!("email_verification_tokens", "used_at IS NULL", "user_id"),
    lock_sql: "UPDATE email_verification_tokens SET used_at = $1 \
               WHERE selector = $2 AND used_at IS NULL AND attempt_count >= $3",
    claim_sql: "UPDATE email_verification_tokens SET used_at = $1 \
                WHERE selector = $2 AND used_at IS NULL",
    invalid: invalid_verification_token,
    what: "verification token",
    max_attempts: LINK_TOKEN_MAX_ATTEMPTS,
};

/// Tokens issued for a user since a moment, used or not, for rate limiting.
pub(crate) const COUNT_RECENT_VERIFICATION_TOKENS_SQL: &str = r"
            SELECT COUNT(*) AS cnt
            FROM email_verification_tokens
            WHERE user_id = $1
              AND created_at >= $2
            ";

/// Stamp the proof. `IS NULL` keeps the first proof's timestamp — re-verifying
/// later (a second link from the same inbox) must not rewrite when it happened.
pub(crate) const MARK_EMAIL_VERIFIED_SQL: &str =
    "UPDATE users SET email_verified_at = $1 WHERE id = $2 AND email_verified_at IS NULL";

/// Whether the address has been proven, as a boolean on both drivers rather
/// than the timestamp column itself, which `SQLite` stores as text and Postgres
/// as `TIMESTAMPTZ`.
pub(crate) const IS_EMAIL_VERIFIED_SQL: &str =
    "SELECT email_verified_at IS NOT NULL AS verified FROM users WHERE id = $1";

/// One uniform error for every failure mode (unknown selector, expired, wrong
/// verifier, locked out) so the endpoint never reveals which hit.
pub(crate) fn invalid_verification_token() -> AppError {
    AppError::not_found("Verification link is invalid, expired, or already used")
}

/// Emit the whole [`EmailVerificationRepository`](super::EmailVerificationRepository)
/// implementation for one backend type.
///
/// `$ids` is that backend's [`uuid_columns`](super::uuid_columns) codec, which
/// spells how the `user_id` column binds and reads. The body is written once
/// here; each backend's shell invokes it with its own type, and sqlx resolves
/// the driver from `self.pool()` per expansion.
macro_rules! impl_email_verification_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl EmailVerificationRepository for $ty {
            async fn store_token(
                &self,
                user_id: Uuid,
                selector: &str,
                verifier_hash: &str,
                ttl_minutes: i64,
            ) -> AppResult<Uuid> {
                let id = Uuid::new_v4();
                let now = Utc::now();
                let expires_at = now + chrono::Duration::minutes(ttl_minutes);

                sqlx::query(STORE_VERIFICATION_TOKEN_SQL)
                    .bind(id.to_string())
                    .bind($ids::bind(user_id))
                    .bind(selector)
                    .bind(verifier_hash)
                    .bind(expires_at)
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store verification token: {e}"))
                    })?;

                Ok(id)
            }

            async fn consume_token(&self, selector: &str, verifier_hash: &str) -> AppResult<Uuid> {
                let row = consume_link_token!(
                    self.pool(),
                    &VERIFICATION_LINK_TOKEN,
                    selector,
                    verifier_hash
                )?;
                $ids::read(&row, "user_id")
            }

            async fn count_recent_tokens(
                &self,
                user_id: Uuid,
                since: DateTime<Utc>,
            ) -> AppResult<i64> {
                let row = sqlx::query(COUNT_RECENT_VERIFICATION_TOKENS_SQL)
                    .bind($ids::bind(user_id))
                    .bind(since)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to count recent verification tokens: {e}"
                        ))
                    })?;

                row.try_get::<i64, _>("cnt")
                    .map_err(|e| AppError::database(format!("Failed to get cnt: {e}")))
            }

            async fn mark_verified(&self, user_id: Uuid) -> AppResult<bool> {
                let done = sqlx::query(MARK_EMAIL_VERIFIED_SQL)
                    .bind(Utc::now())
                    .bind($ids::bind(user_id))
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to mark email verified: {e}"))
                    })?;

                Ok(done.rows_affected() > 0)
            }

            async fn is_verified(&self, user_id: Uuid) -> AppResult<bool> {
                let row = sqlx::query(IS_EMAIL_VERIFIED_SQL)
                    .bind($ids::bind(user_id))
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read verification state: {e}"))
                    })?;

                row.map(|r| {
                    r.try_get::<bool, _>("verified")
                        .map_err(|e| AppError::database(format!("Failed to get verified: {e}")))
                })
                .transpose()
                .map(|verified| verified.unwrap_or(false))
            }
        }
    };
}
pub(crate) use impl_email_verification_repository;
