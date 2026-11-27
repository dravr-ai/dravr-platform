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
use crate::oauth2_server::authorization_details::{is_agent_write_scope, AGENT_WRITE_SCOPE};

impl OAuth2AuthorizationServer {
    /// The scope a token response and a refresh row state for `granted`, the
    /// grant a stored `scope` decided: `pdpp:agent_write` for an agent-write
    /// family, the rendered grant otherwise; `None` for an empty one.
    pub(super) fn minted_scope(granted: &[OAuthScope], stored: Option<&str>) -> Option<String> {
        if is_agent_write_scope(stored) {
            return Some(AGENT_WRITE_SCOPE.to_owned());
        }
        (!granted.is_empty()).then(|| OAuthScope::render_granted(granted))
    }

    /// Generate JWT access token with RS256 asymmetric signing. The grant
    /// rides in the token's `scope` claim.
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
    pub(super) fn generate_access_token(
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
        // An agent-write family's tokens carry its scope alone: it parses to
        // no `OAuthScope`, so the grant itself names no tool (owl PDPP §5.1).
        let scopes: Vec<String> = if lineage.as_ref().is_some_and(|l| l.agent_write) {
            vec![AGENT_WRITE_SCOPE.to_owned()]
        } else {
            granted
                .iter()
                .map(|scope| scope.as_str().to_owned())
                .collect()
        };

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
                tenant,
            ),
            None => self.auth_manager.generate_oauth_access_token(
                &self.jwks_manager,
                &grant,
                &uid,
                tenant,
            ),
        }
        .map_err(|e| AppError::internal(format!("Failed to generate OAuth access token: {e}")))
    }
}
