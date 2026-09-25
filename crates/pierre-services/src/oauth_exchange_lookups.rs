// ABOUTME: What the OAuth callback resolves before storing a token, split out of oauth_flow
// ABOUTME: The user and tenant the token lands under, the registry's token endpoint, WHOOP's owner id

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Lookups the callback needs, kept out of `oauth_flow` so that orchestrator
//! stays within its size budget.
//!
//! The token is stored under the tenant the authorization was made in, which
//! the state pins, so the authorize path's read of the athlete's current app
//! and the exchange's tenant credentials name the same row the callback
//! writes.
//!
//! The token endpoint is the registry's resolved one, so a
//! `PIERRE_<PROVIDER>_TOKEN_URL` override reaches the exchange the same way
//! it reaches refresh and revocation. The owner id is what a provider's push
//! events name the athlete by; Strava returns it inline in the token
//! response, WHOOP only from its profile endpoint, so that one is read with
//! the fresh access token before the token is stored.

use pierre_auth::oauth2_client::OAuth2Token;
use pierre_core::constants::oauth_providers;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::User;
use pierre_providers::whoop_provider::owner_id_for_access_token;
use pierre_providers::OAuthEndpoints;
use tracing::{error, info, warn};

use crate::oauth_flow::OAuthService;

impl OAuthService {
    /// Get user and tenant from database
    ///
    /// The tenant is the one the authorization was made in, pinned on the
    /// state, so the token lands where the authorize path looked for the
    /// athlete's current app and whose credentials the exchange used. A state
    /// that pins none, or pins a tenant the user no longer belongs to, takes
    /// the user's first tenant in `tenant_users`: by the callback the athlete
    /// has already granted access at the provider, and a token dropped here
    /// would leave that grant authorized with nothing to revoke it by.
    pub(crate) async fn get_user_and_tenant(
        &self,
        user_id: uuid::Uuid,
        provider: &str,
        state_tenant: Option<uuid::Uuid>,
    ) -> AppResult<(User, String)> {
        let repos = self.data.repos();
        let user = repos
            .users
            .get_global(user_id)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user: {e}")))?
            .ok_or_else(|| {
                error!(
                    "OAuth callback failed: User not found - user_id: {}, provider: {}",
                    user_id, provider
                );
                AppError::not_found("User")
            })?;

        // The callback carries no session, so the tenant comes from the state
        // the authorize request stored, checked against the memberships.
        let tenants = repos
            .tenants
            .list_for_user(user_id)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user tenants: {e}")))?;
        let pinned = state_tenant.and_then(|pinned| {
            let member = tenants.iter().find(|t| t.id.as_uuid() == pinned);
            if member.is_none() {
                warn!(
                    user_id = %user.id,
                    provider = %provider,
                    tenant_id = %pinned,
                    "OAuth callback: the authorization's tenant is not one of the user's; storing under their first tenant"
                );
            }
            member
        });
        let tenant = pinned.or_else(|| tenants.first()).ok_or_else(|| {
            error!(
                user_id = %user.id,
                provider = %provider,
                "OAuth callback failed: user has no tenant"
            );
            AppError::invalid_input("User has no tenant")
        })?;

        Ok((user, tenant.id.to_string()))
    }

    /// Fill in the provider-side owner id when the token exchange did not
    /// deliver one.
    ///
    /// Strava returns the owner inline in the token response and arrives
    /// here with the id set. WHOOP does not, and its webhooks name the
    /// athlete by that id alone, so the profile is read with the fresh access
    /// token ([`owner_id_for_access_token`]) before the token is stored.
    /// Best-effort: a failed read stores the token without the id — the
    /// connection works, only push-event routing waits — and the refresh path
    /// fills it on the next token refresh.
    pub(crate) async fn with_provider_user_id(
        &self,
        provider: &str,
        user_id: uuid::Uuid,
        mut token: OAuth2Token,
    ) -> OAuth2Token {
        if token.provider_user_id.is_some() || provider != oauth_providers::WHOOP {
            return token;
        }
        match owner_id_for_access_token(self.data.provider_registry(), &token.access_token).await {
            Ok(id) => {
                info!(
                    user_id = %user_id,
                    provider = %provider,
                    "captured provider user id at token exchange"
                );
                token.provider_user_id = Some(id);
            }
            Err(e) => {
                warn!(
                    user_id = %user_id,
                    provider = %provider,
                    error = %e,
                    "provider user id lookup failed at token exchange; push events for this connection route only once a refresh captures it"
                );
            }
        }
        token
    }

    /// The token endpoint a code exchange for `provider` posts to.
    ///
    /// The registry's resolved endpoint: the descriptor's URL after the
    /// `PIERRE_<PROVIDER>_TOKEN_URL` override was applied at registration —
    /// the same resolution the refresh path and provider revocation read, so
    /// an integration test that points the override at a local mock reaches
    /// the exchange too. Production leaves the override unset, where the two
    /// are equal. A provider the registry holds no configuration for posts to
    /// the descriptor's endpoint.
    pub(crate) fn token_url_for(&self, provider: &str, endpoints: &OAuthEndpoints) -> String {
        self.data
            .provider_registry()
            .default_config(provider)
            .map_or_else(
                || endpoints.token_url.to_owned(),
                |config| config.token_url.clone(),
            )
    }
}
