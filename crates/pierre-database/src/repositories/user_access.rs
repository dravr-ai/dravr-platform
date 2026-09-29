// ABOUTME: Repository traits for the credentials around a user account: device refresh tokens and email pre-approvals
// ABOUTME: Implemented per backend beside the user store; re-exported flat from the repositories module
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::{PreApprovedEmail, SessionRefreshToken};
use uuid::Uuid;

/// First-party refresh tokens — the credential a device holds between JWTs.
///
/// A login opens a family; each exchange stores the successor in that family
/// and revokes the token it replaced, so at most one member is live. Tokens
/// are passed in plaintext and stored as their HMAC, the same blind index the
/// `OAuth2` server's refresh tokens use, so a database read cannot replay one.
///
/// Not [`OAuth2ServerRepository`](super::OAuth2ServerRepository)'s refresh
/// tokens: those are keyed to a registered OAuth client and cascade with it,
/// which a password login has no counterpart for.
#[async_trait]
pub trait SessionRefreshTokenRepository: Send + Sync {
    /// Store a freshly issued token under its family.
    async fn store_token(&self, token: &str, record: &SessionRefreshToken) -> AppResult<()>;
    /// Exchange a token: mark it revoked and return its record, in one
    /// statement so two concurrent exchanges cannot both succeed. `None` when
    /// the token is unknown, already revoked, or expired at `now`.
    async fn consume_token(
        &self,
        token: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<SessionRefreshToken>>;
    /// Revoke every live member of the family this token belongs to, and
    /// return how many were revoked. Zero means the token was unknown or its
    /// family was already dead; more than zero after a failed exchange means a
    /// rotated-out token was replayed and its successor is now dead too.
    async fn revoke_token_family(&self, token: &str, now: DateTime<Utc>) -> AppResult<u64>;
    /// Revoke every live token the user holds, on any device — the password
    /// changed, so every session minted under the old one ends.
    async fn revoke_user_tokens(&self, user_id: Uuid, now: DateTime<Utc>) -> AppResult<u64>;
}

/// Standing per-email pre-approvals — an operator "allow" recorded before the
/// person has an account.
///
/// The registration approval decision consults this list so an allowed address
/// lands `Active` without the pending queue; `pierre-cli user allow / disallow /
/// list-allowed` manages it. Implementations store emails normalized (trimmed,
/// lowercase) and compare them lower-cased, so lookups are case-insensitive.
#[async_trait]
pub trait PreApprovedEmailRepository: Send + Sync {
    /// Record an allow for `email`. Idempotent: returns `false` when the
    /// address was already on the list (the original row is kept).
    async fn allow(
        &self,
        email: &str,
        allowed_by: Option<Uuid>,
        note: Option<&str>,
    ) -> AppResult<bool>;
    /// Remove the allow for `email`. Returns `false` when none existed.
    async fn remove(&self, email: &str) -> AppResult<bool>;
    /// Fetch the allow for `email`, if present.
    async fn get(&self, email: &str) -> AppResult<Option<PreApprovedEmail>>;
    /// Every standing allow, oldest first.
    async fn list(&self) -> AppResult<Vec<PreApprovedEmail>>;
}
