// ABOUTME: Repository trait, shared statements and body for dravr.ai docs sign-in tokens — issue, consume once, rate-limit
// ABOUTME: One SQL text per operation; each backend shell supplies only the uuid codec its user_id column needs

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Website sign-in tokens, written once.
//!
//! The magic link that opens the members part of the dravr.ai docs. It uses
//! the `<selector>.<verifier>` mechanism of the email-verification flow
//! against a separate token space: only the SHA-256 of the verifier is stored,
//! a wrong guess costs one attempt without consuming the token, and past the
//! attempt cap the token self-invalidates. A sign-in token must never verify
//! an address, nor a verification token open a docs session, so the two never
//! share a table.
//!
//! As with the verification tokens, `user_id` is a `uuid` column on Postgres
//! and `TEXT` on `SQLite`, so the shell hands the body its
//! [`uuid_columns`](super::uuid_columns) codec; timestamps bind as
//! [`DateTime<Utc>`] on both, and `attempt_count` is cast to `BIGINT` so one
//! decode serves both drivers.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use uuid::Uuid;

/// Max wrong verifier guesses against one sign-in token before it
/// self-invalidates. Same posture as the verification and reset flows (CWE-307).
pub(crate) const SIGN_IN_MAX_ATTEMPTS: i64 = 5;

/// Docs sign-in links for the dravr.ai website.
#[async_trait]
pub trait WebsiteSignInTokenRepository: Send + Sync {
    /// Store one issued token for `user_id`, with a TTL in minutes.
    ///
    /// `selector` is the plaintext lookup half and `verifier_hash` the SHA-256
    /// of the secret half; the delivered token is never stored whole.
    async fn store_token(
        &self,
        user_id: Uuid,
        selector: &str,
        verifier_hash: &str,
        ttl_minutes: i64,
    ) -> AppResult<()>;
    /// Claim a token single-use, returning the user it signs in.
    ///
    /// Every failure (unknown, expired, used, wrong verifier, locked out)
    /// answers the same error, so the caller cannot tell which it hit.
    async fn consume_token(&self, selector: &str, verifier_hash: &str) -> AppResult<Uuid>;
    /// Count tokens issued for a user since `since`, used or not, for the send budget.
    async fn count_recent_tokens(&self, user_id: Uuid, since: DateTime<Utc>) -> AppResult<i64>;
}

/// Issue one token: the selector half in clear, the verifier half hashed.
pub(crate) const STORE_SIGN_IN_TOKEN_SQL: &str = r"
            INSERT INTO website_sign_in_tokens (id, user_id, selector, token_hash, expires_at, created_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ";

/// The live token behind a selector: unused and not yet expired at `$2`.
pub(crate) const LOAD_SIGN_IN_TOKEN_SQL: &str = r"
            SELECT user_id, token_hash, CAST(attempt_count AS BIGINT) AS attempt_count
            FROM website_sign_in_tokens
            WHERE selector = $1
              AND used_at IS NULL
              AND expires_at > $2
            ";

/// Brute-force lockout: past the attempt cap the token is spent outright.
pub(crate) const LOCK_SIGN_IN_TOKEN_SQL: &str =
    "UPDATE website_sign_in_tokens SET used_at = $1 WHERE selector = $2";

/// A wrong verifier costs one attempt and leaves the token live.
pub(crate) const RECORD_SIGN_IN_ATTEMPT_SQL: &str =
    "UPDATE website_sign_in_tokens SET attempt_count = attempt_count + 1 WHERE selector = $1";

/// Claim the token exactly once: only the request that flips `used_at` from
/// NULL affects a row, so a concurrent request holding the same live row loses.
pub(crate) const CONSUME_SIGN_IN_TOKEN_SQL: &str =
    "UPDATE website_sign_in_tokens SET used_at = $1 WHERE selector = $2 AND used_at IS NULL";

/// Tokens issued for a user since a moment, used or not.
pub(crate) const COUNT_RECENT_SIGN_IN_TOKENS_SQL: &str = r"
            SELECT COUNT(*) AS cnt
            FROM website_sign_in_tokens
            WHERE user_id = $1
              AND created_at >= $2
            ";

/// One uniform error for every failure mode, so the endpoint never reveals
/// which one hit.
#[must_use]
pub fn invalid_sign_in_token() -> AppError {
    AppError::not_found("Sign-in link is invalid, expired, or already used")
}

/// Emit the whole [`WebsiteSignInTokenRepository`] implementation for one
/// backend type. `$ids` is that backend's [`uuid_columns`](super::uuid_columns)
/// codec. The body names its consts, helpers and types unqualified, so the
/// invoking shell must `use` every one of them.
macro_rules! impl_website_sign_in_token_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl WebsiteSignInTokenRepository for $ty {
            async fn store_token(
                &self,
                user_id: Uuid,
                selector: &str,
                verifier_hash: &str,
                ttl_minutes: i64,
            ) -> AppResult<()> {
                let now = Utc::now();
                let expires_at = now + chrono::Duration::minutes(ttl_minutes);

                sqlx::query(STORE_SIGN_IN_TOKEN_SQL)
                    .bind(Uuid::new_v4().to_string())
                    .bind($ids::bind(user_id))
                    .bind(selector)
                    .bind(verifier_hash)
                    .bind(expires_at)
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to store sign-in token: {e}"))
                    })?;
                Ok(())
            }

            async fn consume_token(&self, selector: &str, verifier_hash: &str) -> AppResult<Uuid> {
                let now = Utc::now();

                let row = sqlx::query(LOAD_SIGN_IN_TOKEN_SQL)
                    .bind(selector)
                    .bind(now)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to load sign-in token: {e}"))
                    })?;

                let Some(row) = row else {
                    return Err(invalid_sign_in_token());
                };
                let user_id = $ids::read(&row, "user_id")?;
                let stored_hash: String = row
                    .try_get("token_hash")
                    .map_err(|e| AppError::database(format!("Failed to get token_hash: {e}")))?;
                let attempts: i64 = row
                    .try_get("attempt_count")
                    .map_err(|e| AppError::database(format!("Failed to get attempt_count: {e}")))?;

                if attempts >= SIGN_IN_MAX_ATTEMPTS {
                    sqlx::query(LOCK_SIGN_IN_TOKEN_SQL)
                        .bind(now)
                        .bind(selector)
                        .execute(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to lock sign-in token: {e}"))
                        })?;
                    return Err(invalid_sign_in_token());
                }

                // Comparing SHA-256 hashes, not the secret: a timing side-channel on the
                // hash cannot forge the verifier without a preimage.
                if stored_hash != verifier_hash {
                    sqlx::query(RECORD_SIGN_IN_ATTEMPT_SQL)
                        .bind(selector)
                        .execute(self.pool())
                        .await
                        .map_err(|e| {
                            AppError::database(format!("Failed to record sign-in attempt: {e}"))
                        })?;
                    return Err(invalid_sign_in_token());
                }

                let claimed = sqlx::query(CONSUME_SIGN_IN_TOKEN_SQL)
                    .bind(now)
                    .bind(selector)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to consume sign-in token: {e}"))
                    })?;

                if claimed.rows_affected() == 0 {
                    return Err(invalid_sign_in_token());
                }

                Ok(user_id)
            }

            async fn count_recent_tokens(
                &self,
                user_id: Uuid,
                since: DateTime<Utc>,
            ) -> AppResult<i64> {
                let row = sqlx::query(COUNT_RECENT_SIGN_IN_TOKENS_SQL)
                    .bind($ids::bind(user_id))
                    .bind(since)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to count recent sign-in tokens: {e}"))
                    })?;

                row.try_get::<i64, _>("cnt")
                    .map_err(|e| AppError::database(format!("Failed to get cnt: {e}")))
            }
        }
    };
}
pub(crate) use impl_website_sign_in_token_repository;
