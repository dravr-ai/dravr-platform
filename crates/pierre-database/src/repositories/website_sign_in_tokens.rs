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
//! [`DateTime<Utc>`] on both. The consume runs through the one body every link token shares
//! ([`link_token_consume`](super::link_token_consume)), which binds the
//! attempt cap against the `INTEGER` `attempt_count` column on both drivers.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use uuid::Uuid;

use super::link_token_consume::{link_token_guess_sql, LinkTokenTable, LINK_TOKEN_MAX_ATTEMPTS};

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

/// How a docs sign-in token is consumed ([`consume_link_token`]).
pub(crate) const SIGN_IN_LINK_TOKEN: LinkTokenTable = LinkTokenTable {
    guess_sql: link_token_guess_sql!("website_sign_in_tokens", "used_at IS NULL", "user_id"),
    lock_sql: "UPDATE website_sign_in_tokens SET used_at = $1 \
               WHERE selector = $2 AND used_at IS NULL AND attempt_count >= $3",
    claim_sql: "UPDATE website_sign_in_tokens SET used_at = $1 \
                WHERE selector = $2 AND used_at IS NULL",
    invalid: invalid_sign_in_token,
    what: "sign-in token",
    max_attempts: LINK_TOKEN_MAX_ATTEMPTS,
};

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
                let row =
                    consume_link_token!(self.pool(), &SIGN_IN_LINK_TOKEN, selector, verifier_hash)?;
                $ids::read(&row, "user_id")
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
