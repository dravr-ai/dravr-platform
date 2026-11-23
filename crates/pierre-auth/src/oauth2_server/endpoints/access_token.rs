// ABOUTME: Minting the access token a grant is redeemed for: a user's under its refresh family, or a client's own
// ABOUTME: A user token carries the family as grant_id, the grant an admin's support session is bound to (carnet#1511)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::permissions::scopes::OAuthScope;
use tracing::debug;
use uuid::Uuid;

use super::OAuth2AuthorizationServer;
use crate::auth::{DelegatedGrant, TokenLineage};

impl OAuth2AuthorizationServer {
    /// Generate JWT access token with RS256 asymmetric signing.
    ///
    /// Async because a user-bound token has to carry the athlete's real
    /// connected providers. That field used to be filled with the granted
    /// scopes, which is how an OAuth-minted token came to report `fitness:read`
    /// as a connected provider; the grant now rides in the token's own `scope`
    /// claim and `providers` means what it means everywhere else.
    ///
    /// `audience` is the RFC 8707 resource the token is bound to, `None` for
    /// the platform audience. `subject` is the user the grant acts for and the
    /// tenant it was authorized in, which the caller confirmed the user still
    /// belongs to ([`Self::require_grant_tenant`]); the token carries it as
    /// `active_tenant_id`. A user token is never minted without one.
    ///
    /// `lineage` is the refresh family a user token is minted in, which it
    /// carries as its `grant_id` claim (the grant an admin's support session
    /// is bound to, carnet#1511), and the consent grant the family was given
    /// under, its `client_grant_id` claim. The code grant starts the family
    /// and the refresh grant keeps both.
    pub(super) async fn generate_access_token(
        &self,
        client_id: &str,
        subject: Option<(Uuid, &str)>,
        lineage: Option<TokenLineage<'_>>,
        granted: &[OAuthScope],
        audience: Option<&str>,
    ) -> AppResult<String> {
        if granted.is_empty() {
            debug!(
                client_id = %client_id,
                user_id = ?subject.map(|(user_id, _)| user_id),
                "Minting an access token with an empty grant"
            );
        }
        let scopes: Vec<String> = granted
            .iter()
            .map(|scope| scope.as_str().to_owned())
            .collect();

        let Some((uid, tenant_id)) = subject else {
            return self
                .auth_manager
                .generate_client_credentials_token(
                    &self.jwks_manager,
                    client_id,
                    &scopes,
                    None, // tenant_id for client credentials
                    audience,
                )
                .map_err(|e| {
                    AppError::internal(format!("Failed to generate client credentials token: {e}"))
                });
        };

        // SECURITY: Global lookup — OAuth2 token minting, no tenant context.
        // A user who cannot be read has no providers to report; the token is
        // still minted, because the grant is what authorizes it and the
        // provider list is descriptive.
        let providers = self
            .users
            .get_global(uid)
            .await
            .ok()
            .flatten()
            .map_or_default(|user| user.available_providers());
        let grant = DelegatedGrant {
            client_id,
            scopes: &scopes,
            audience,
        };
        let tenant = Some(tenant_id.to_owned());
        match lineage {
            Some(lineage) => self.auth_manager.generate_oauth_access_token_in_grant(
                &self.jwks_manager,
                &grant,
                &lineage,
                &uid,
                &providers,
                tenant,
            ),
            None => self.auth_manager.generate_oauth_access_token(
                &self.jwks_manager,
                &grant,
                &uid,
                &providers,
                tenant,
            ),
        }
        .map_err(|e| AppError::internal(format!("Failed to generate OAuth access token: {e}")))
    }
}
