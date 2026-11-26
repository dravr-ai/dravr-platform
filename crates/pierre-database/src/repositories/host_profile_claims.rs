// ABOUTME: Repository trait, statements and shared body for the per-request Host Profile claims a step-up stands on
// ABOUTME: Written by /mcp when a call a step-up would cure declares T2, read by the step-up approval; emitted per backend

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Per-request Host Profile claims (carnet#1544 slice 4b, owl PDPP §6.2
//! `declared_request`).
//!
//! A client with no metadata document declares its Host Profile only on its
//! own MCP requests. owl requires the claim be seen on the request that
//! triggered consent, and the authorize request that follows carries no MCP
//! `_meta`: so `/mcp` records the claim of a call a step-up would cure, under
//! the live parent grant the call was made under, and the step-up approval
//! reads it. One row per parent, replaced by the next claim seen.
//!
//! The write takes the tenant, the user and the client from the parent row
//! itself, and only while it is a live parent of the calling user; every
//! read filters `tenant_id` and `user_id`.

use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::HostProfileClaim;

/// The per-request Host Profile claims a step-up stands on.
#[async_trait]
pub trait HostProfileClaimRepository: Send + Sync {
    /// Record `claim`, seen on a call `user_id` made under its
    /// `client_grant_id`, replacing the claim seen before under that grant.
    /// Returns whether it was recorded: nothing is when that grant is not a
    /// live parent grant of `user_id`.
    async fn record_host_profile_claim(
        &self,
        user_id: &str,
        claim: &HostProfileClaim,
    ) -> AppResult<bool>;

    /// The latest claim seen under grant `client_grant_id` of `user_id` in
    /// `tenant_id`; `None` when none was.
    async fn host_profile_claim(
        &self,
        tenant_id: &str,
        user_id: &str,
        client_grant_id: &str,
    ) -> AppResult<Option<HostProfileClaim>>;
}

/// Record a claim under a live parent grant of the calling user, its tenant,
/// user and client read from the grant row.
pub(crate) const RECORD_HOST_PROFILE_CLAIM_SQL: &str = r"
    INSERT INTO oauth_host_profile_claims
        (client_grant_id, tenant_id, user_id, client_id, tier_max, seen_at)
    SELECT id, tenant_id, user_id, client_id, $3, $4
    FROM oauth_client_grants
    WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL AND parent_grant_id IS NULL
    ON CONFLICT (client_grant_id) DO UPDATE
        SET tier_max = excluded.tier_max, seen_at = excluded.seen_at
";

/// The claim seen under one grant of one user in one tenant.
pub(crate) const GET_HOST_PROFILE_CLAIM_SQL: &str = r"
    SELECT client_grant_id, tier_max, seen_at
    FROM oauth_host_profile_claims
    WHERE tenant_id = $1 AND user_id = $2 AND client_grant_id = $3
";

/// Emit the whole [`HostProfileClaimRepository`] implementation for one
/// backend type; sqlx resolves the driver from `self.pool()`.
macro_rules! impl_host_profile_claim_repository {
    ($ty:ty) => {
        #[async_trait::async_trait]
        impl HostProfileClaimRepository for $ty {
            async fn record_host_profile_claim(
                &self,
                user_id: &str,
                claim: &HostProfileClaim,
            ) -> AppResult<bool> {
                let recorded = sqlx::query(RECORD_HOST_PROFILE_CLAIM_SQL)
                    .bind(&claim.client_grant_id)
                    .bind(user_id)
                    .bind(&claim.tier_max)
                    .bind(claim.seen_at)
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to record a Host Profile claim: {e}"))
                    })?;
                Ok(recorded.rows_affected() > 0)
            }

            async fn host_profile_claim(
                &self,
                tenant_id: &str,
                user_id: &str,
                client_grant_id: &str,
            ) -> AppResult<Option<HostProfileClaim>> {
                let row = sqlx::query(GET_HOST_PROFILE_CLAIM_SQL)
                    .bind(tenant_id)
                    .bind(user_id)
                    .bind(client_grant_id)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to read a Host Profile claim: {e}"))
                    })?;
                row.map(|row| {
                    let column = |name: &str, e: sqlx::Error| {
                        AppError::database(format!(
                            "Failed to read oauth_host_profile_claims.{name}: {e}"
                        ))
                    };
                    Ok(HostProfileClaim {
                        client_grant_id: row
                            .try_get("client_grant_id")
                            .map_err(|e| column("client_grant_id", e))?,
                        tier_max: row.try_get("tier_max").map_err(|e| column("tier_max", e))?,
                        seen_at: row.try_get("seen_at").map_err(|e| column("seen_at", e))?,
                    })
                })
                .transpose()
            }
        }
    };
}
pub(crate) use impl_host_profile_claim_repository;
