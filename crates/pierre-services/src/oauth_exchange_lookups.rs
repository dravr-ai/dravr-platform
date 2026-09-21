// ABOUTME: What the token exchange resolves through the provider registry, split out of oauth_flow
// ABOUTME: The registry's token endpoint for a provider, and WHOOP's owner id read from the profile

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Two registry-backed lookups the code exchange needs, kept out of
//! `oauth_flow` so that orchestrator stays within its size budget.
//!
//! The token endpoint is the registry's resolved one, so a
//! `PIERRE_<PROVIDER>_TOKEN_URL` override reaches the exchange the same way
//! it reaches refresh and revocation. The owner id is what a provider's push
//! events name the athlete by; Strava and Fitbit return it inline in the
//! token response, WHOOP only from its profile endpoint, so that one is read
//! with the fresh access token before the token is stored.

use pierre_auth::oauth2_client::OAuth2Token;
use pierre_core::constants::oauth_providers;
use pierre_providers::whoop_provider::owner_id_for_access_token;
use pierre_providers::OAuthEndpoints;
use tracing::{info, warn};

use crate::oauth_flow::OAuthService;

impl OAuthService {
    /// Fill in the provider-side owner id when the token exchange did not
    /// deliver one.
    ///
    /// Strava and Fitbit return the owner inline in the token response and
    /// arrive here with the id set. WHOOP does not, and its webhooks name the
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
