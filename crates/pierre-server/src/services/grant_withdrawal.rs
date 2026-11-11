// ABOUTME: The disconnect chokepoint for a withdrawn OAuth grant, telling a grant-only athlete to sign in again
// ABOUTME: Wraps OAuthService for the seat reclaimer and the Strava deauthorization webhook (carnet#1494)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A grant withdrawn without the athlete asking.
//!
//! A seat reclaim and the provider's own revocation clear the one grant they
//! name and keep the athlete's sign-in session of the same provider
//! (`OAuthService::disconnect_provider`). An athlete that grant served alone
//! is then left with nothing, so they are told once, through the hosted
//! sign-in link every reconnect carries
//! ([`notify_grant_withdrawn`]). The withdrawal is the notice's claim: it is
//! sent only by the disconnect that found the grant's token row and removed
//! it, so a redelivered event or a retried pass finds nothing and sends
//! nothing.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_services::oauth_flow::OAuthService;
use pierre_services::provider_revocation::{DisconnectReason, RevocationOutcome};
use pierre_services::user_removal::ProviderDisconnector;
use pierre_tool_runtime::protocol::reauth_notice::notify_grant_withdrawn;
use pierre_tool_runtime::runtime::ToolRuntime;
use tracing::info;
use uuid::Uuid;

/// [`OAuthService`]'s disconnect, followed by the reconnect notice of a
/// grant-only athlete whose grant a seat reclaim or a revocation withdrew.
pub struct GrantWithdrawal {
    service: OAuthService,
    runtime: Arc<dyn ToolRuntime>,
}

impl fmt::Debug for GrantWithdrawal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GrantWithdrawal").finish_non_exhaustive()
    }
}

impl GrantWithdrawal {
    /// The chokepoint over `service`, sending its notices through `runtime`.
    #[must_use]
    pub const fn new(service: OAuthService, runtime: Arc<dyn ToolRuntime>) -> Self {
        Self { service, runtime }
    }
}

/// Whether `reason` withdraws a grant without the athlete asking, so the
/// athlete it served alone is owed a reconnect.
const fn withdraws_without_asking(reason: DisconnectReason) -> bool {
    matches!(
        reason,
        DisconnectReason::SeatReclaim | DisconnectReason::ProviderRevoked
    )
}

#[async_trait]
impl ProviderDisconnector for GrantWithdrawal {
    async fn disconnect(
        &self,
        user_id: Uuid,
        provider: &str,
        tenant_id: TenantId,
        reason: DisconnectReason,
    ) -> AppResult<RevocationOutcome> {
        // Read before the delete: only the disconnect that removes the grant
        // sends its notice. A row that cannot be read sends none.
        let held = withdraws_without_asking(reason)
            && matches!(
                self.runtime
                    .repos()
                    .oauth_tokens
                    .get_token(user_id, tenant_id, provider)
                    .await,
                Ok(Some(_))
            );
        let outcome = self
            .service
            .disconnect(user_id, provider, tenant_id, reason)
            .await?;
        if held {
            let notice = notify_grant_withdrawn(&self.runtime, user_id, tenant_id, provider).await;
            info!(
                user_id = %user_id,
                provider = %provider,
                reason = reason.as_str(),
                told = notice.claimed,
                chat_channels = notice.chat_channels,
                "Grant withdrawn: the reconnect notice of a grant-only athlete"
            );
        }
        Ok(outcome)
    }

    async fn withdraw_upstream(
        &self,
        user_id: Uuid,
        provider: &str,
        tenant_id: TenantId,
    ) -> RevocationOutcome {
        self.service
            .withdraw_upstream(user_id, provider, tenant_id)
            .await
    }

    fn supports(&self, provider: &str) -> bool {
        self.service.supports(provider)
    }
}
