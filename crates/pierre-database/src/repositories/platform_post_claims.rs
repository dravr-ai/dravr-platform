// ABOUTME: Repository trait for the once-claim on a post the coach makes to an athlete on its own initiative
// ABOUTME: The SQL is written once here and both backends emit their impl from it with their own uuid codec

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Which server-initiated coach posts an athlete has had.
//!
//! The coach speaks first once an athlete's data lands (carnet#831), and that
//! post is owed once per purpose: a second provider, a reconnect and the same
//! backfill finishing on two replicas all fire the same trigger. The trigger
//! *claims* `(tenant, athlete, purpose)` before it spends a model call; the
//! primary key decides which of any racing triggers posts. A trigger that
//! ended up posting nothing releases its claim, so the next landing tries
//! again, and one that posted marks it posted
//! ([`PlatformPostClaimRepository::mark_platform_post_posted`]). A claim
//! neither released nor marked was abandoned — the instance holding it
//! stopped mid-post — and is retaken once its lease has passed
//! ([`PlatformPostClaimRepository::claim_platform_post_once`], carnet#911).
//!
//! A check-in is owed once per period rather than once ever, so its claim is
//! the same row taken again once the period it was last taken in has passed
//! ([`PlatformPostClaimRepository::claim_platform_post_since`]): replicas
//! racing on one scheduler tick still post once, and the athlete keeps one
//! row however long they are checked in on.
//!
//! Every statement is keyed by `tenant_id` and `user_id` together.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use uuid::Uuid;

/// The once-claims on the coach's own posts.
#[async_trait]
pub trait PlatformPostClaimRepository: Send + Sync {
    /// Claim the post for `purpose` to `user_id` in `tenant_id`, owed once
    /// ever, at `claimed_at`.
    ///
    /// Returns `true` when this call took the claim: none was held, or the one
    /// held never posted and was taken before `lease_expired_before`, so the
    /// trigger holding it is presumed gone. `false` when the post was made,
    /// or a trigger took the claim within its lease and is making it.
    async fn claim_platform_post_once(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        purpose: &str,
        lease_expired_before: DateTime<Utc>,
        claimed_at: DateTime<Utc>,
    ) -> AppResult<bool>;

    /// Record that the post for `purpose` to `user_id` in `tenant_id` landed,
    /// at `posted_at`, so its claim is never retaken. Returns whether a claim
    /// was marked.
    async fn mark_platform_post_posted(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        purpose: &str,
        posted_at: DateTime<Utc>,
    ) -> AppResult<bool>;

    /// Claim the post for `purpose` to `user_id` in `tenant_id`, at
    /// `claimed_at`, unless it was already claimed at or after `since` — the
    /// start of the period the post is owed for.
    ///
    /// Returns `true` when this call took the claim: no claim was held, or
    /// the one held dates from an earlier period. `false` when a trigger
    /// already claimed it this period.
    async fn claim_platform_post_since(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        purpose: &str,
        since: DateTime<Utc>,
        claimed_at: DateTime<Utc>,
    ) -> AppResult<bool>;

    /// Release the claim for `purpose` to `user_id` in `tenant_id` after the
    /// trigger posted nothing, so the next trigger may post. Returns whether a
    /// claim was released.
    async fn release_platform_post(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        purpose: &str,
    ) -> AppResult<bool>;
}

/// The primary key is the claim: a row an earlier trigger inserted is left
/// alone unless it never posted and was taken before `$5`, and the affected
/// count says whether this call took it. `excluded` names the proposed row on
/// both engines.
pub(crate) const CLAIM_PLATFORM_POST_ONCE_SQL: &str = r"
            INSERT INTO platform_post_claims (tenant_id, user_id, purpose, claimed_at_ms)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (tenant_id, user_id, purpose) DO UPDATE
                SET claimed_at_ms = excluded.claimed_at_ms
                WHERE platform_post_claims.posted_at_ms IS NULL
                  AND platform_post_claims.claimed_at_ms < $5
            ";

/// Mark the claim's post landed at `$4`, so the once-claim never retakes it.
pub(crate) const MARK_PLATFORM_POST_POSTED_SQL: &str = r"
            UPDATE platform_post_claims
            SET posted_at_ms = $4
            WHERE tenant_id = $1 AND user_id = $2 AND purpose = $3
            ";

/// The same row, retaken once the claim it holds predates `$5`: the update's
/// condition is the claim, and the affected count says whether this call took
/// it. `excluded` names the proposed row on both engines.
pub(crate) const CLAIM_PLATFORM_POST_SINCE_SQL: &str = r"
            INSERT INTO platform_post_claims (tenant_id, user_id, purpose, claimed_at_ms)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (tenant_id, user_id, purpose) DO UPDATE
                SET claimed_at_ms = excluded.claimed_at_ms
                WHERE platform_post_claims.claimed_at_ms < $5
            ";

pub(crate) const RELEASE_PLATFORM_POST_SQL: &str = r"
            DELETE FROM platform_post_claims
            WHERE tenant_id = $1 AND user_id = $2 AND purpose = $3
            ";

/// Emit the whole [`PlatformPostClaimRepository`] implementation for one
/// backend type. `$ids` is that backend's uuid codec
/// ([`crate::repositories::uuid_columns`]) for the tenant and user columns.
macro_rules! impl_platform_post_claim_repository {
    ($ty:ty, $ids:ident) => {
        #[async_trait::async_trait]
        impl PlatformPostClaimRepository for $ty {
            async fn claim_platform_post_once(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                purpose: &str,
                lease_expired_before: DateTime<Utc>,
                claimed_at: DateTime<Utc>,
            ) -> AppResult<bool> {
                let result = sqlx::query(CLAIM_PLATFORM_POST_ONCE_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .bind(purpose)
                    .bind(claimed_at.timestamp_millis())
                    .bind(lease_expired_before.timestamp_millis())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to claim the platform post: {e}"))
                    })?;
                Ok(result.rows_affected() == 1)
            }

            async fn mark_platform_post_posted(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                purpose: &str,
                posted_at: DateTime<Utc>,
            ) -> AppResult<bool> {
                let result = sqlx::query(MARK_PLATFORM_POST_POSTED_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .bind(purpose)
                    .bind(posted_at.timestamp_millis())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to mark the platform post posted: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn claim_platform_post_since(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                purpose: &str,
                since: DateTime<Utc>,
                claimed_at: DateTime<Utc>,
            ) -> AppResult<bool> {
                let result = sqlx::query(CLAIM_PLATFORM_POST_SINCE_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .bind(purpose)
                    .bind(claimed_at.timestamp_millis())
                    .bind(since.timestamp_millis())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!(
                            "Failed to claim the periodic platform post: {e}"
                        ))
                    })?;
                Ok(result.rows_affected() == 1)
            }

            async fn release_platform_post(
                &self,
                tenant_id: TenantId,
                user_id: Uuid,
                purpose: &str,
            ) -> AppResult<bool> {
                let result = sqlx::query(RELEASE_PLATFORM_POST_SQL)
                    .bind($ids::bind(tenant_id.as_uuid()))
                    .bind($ids::bind(user_id))
                    .bind(purpose)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to release the platform post: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }
        }
    };
}
pub(crate) use impl_platform_post_claim_repository;
