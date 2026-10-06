// ABOUTME: Single-use token repositories of the user domain: password reset and email verification
// ABOUTME: Both store a <selector>.<verifier> token split, never the whole token
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use uuid::Uuid;

/// Password reset token management repository
#[async_trait]
pub trait PasswordResetRepository: Send + Sync {
    /// Store a password reset token (hashed) for a user
    /// Store a reset token's `selector` (plaintext lookup half) and `verifier_hash`
    /// (SHA-256 of the secret half). The delivered token is `<selector>.<verifier>`.
    async fn store_token(
        &self,
        user_id: Uuid,
        selector: &str,
        verifier_hash: &str,
        created_by: &str,
    ) -> AppResult<Uuid>;
    /// Store a password reset token with a custom TTL (in minutes)
    ///
    /// Used for self-service password reset codes that expire faster (15 min)
    /// than admin-issued tokens (1 hour).
    async fn store_token_with_ttl(
        &self,
        user_id: Uuid,
        selector: &str,
        verifier_hash: &str,
        created_by: &str,
        ttl_minutes: i64,
    ) -> AppResult<Uuid>;
    /// Consume a reset token: look it up by `selector`, verify `verifier_hash`, and on
    /// success mark it used and return the user id. A wrong verifier increments a
    /// per-token attempt counter; past the attempt cap the token self-invalidates
    /// (brute-force lockout).
    async fn consume_token(&self, selector: &str, verifier_hash: &str) -> AppResult<Uuid>;
    /// Invalidate all unused reset tokens for a user
    async fn invalidate_tokens(&self, user_id: Uuid) -> AppResult<()>;
    /// Count recent reset tokens for a user (for rate limiting)
    ///
    /// Returns the number of tokens created for the user since the given timestamp,
    /// regardless of whether they have been used or expired.
    async fn count_recent_tokens(&self, user_id: Uuid, since: DateTime<Utc>) -> AppResult<i64>;
}

/// Email-verification token lifecycle — proving an address belongs to whoever typed it.
///
/// Shares the `<selector>.<verifier>` mechanism with [`PasswordResetRepository`]
/// but deliberately not its token space: a token that can reset a password and a
/// token that can verify an address are different capabilities, and invalidating
/// one set must never clear the other.
#[async_trait]
pub trait EmailVerificationRepository: Send + Sync {
    /// Store one half of a verification token, with a TTL in minutes.
    ///
    /// `selector` is the plaintext lookup half and `verifier_hash` the SHA-256
    /// of the secret half. The delivered token is `<selector>.<verifier>` and is
    /// never stored whole.
    async fn store_token(
        &self,
        user_id: Uuid,
        selector: &str,
        verifier_hash: &str,
        ttl_minutes: i64,
    ) -> AppResult<Uuid>;
    /// Claim a verification token single-use, returning the user it proves.
    ///
    /// Looks the token up by `selector` and checks `verifier_hash`. A wrong
    /// verifier costs one attempt without consuming the token; past the attempt
    /// cap the token self-invalidates (brute-force lockout).
    async fn consume_token(&self, selector: &str, verifier_hash: &str) -> AppResult<Uuid>;
    /// Count tokens issued for a user since `since`, for rate limiting.
    async fn count_recent_tokens(&self, user_id: Uuid, since: DateTime<Utc>) -> AppResult<i64>;
    /// Stamp `users.email_verified_at`. Idempotent — a second call leaves the
    /// original timestamp in place, so re-verifying never rewrites history.
    async fn mark_verified(&self, user_id: Uuid) -> AppResult<()>;
    /// Whether this user's address has been proven.
    async fn is_verified(&self, user_id: Uuid) -> AppResult<bool>;
}
