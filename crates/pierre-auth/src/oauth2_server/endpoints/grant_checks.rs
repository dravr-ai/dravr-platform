// ABOUTME: What a grant must still hold before it mints: its resource server, and its tenant membership
// ABOUTME: A grant bound to one resource server never mints for another; a tenant the user left ends the grant; each user's redemptions are metered before one is spent
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::permissions::scopes::OAuthScope;
use tracing::{error, warn};
use uuid::Uuid;

use super::OAuth2AuthorizationServer;
use crate::oauth2_server::models::{OAuth2Error, OAuth2RefreshToken};
use crate::oauth2_server::resource::{token_audience, ResourceServers};
use crate::rate_limiting::OAuth2Endpoint;

/// Whose consent, to which client and in which tenant, a token rides on.
#[derive(Debug, Clone)]
pub struct TokenGrant {
    /// The athlete who consented
    pub user_id: Uuid,
    /// The tenant the consent was given in
    pub tenant_id: String,
    /// The client it was given to
    pub client_id: String,
    /// The scope the token carries, space-delimited
    pub scope: String,
}

impl OAuth2AuthorizationServer {
    /// The audience of a token minted from a grant bound to `granted`, when
    /// the request names `requested`: [`token_audience`] over the platform's
    /// MCP and A2A identifiers and the satellites'.
    ///
    /// # Errors
    /// As [`token_audience`].
    pub(super) fn audience(
        &self,
        requested: Option<&str>,
        granted: Option<&str>,
    ) -> Result<Option<String>, OAuth2Error> {
        let platform: Vec<&str> = self.resources.iter().map(String::as_str).collect();
        let a2a: Vec<&str> = self.a2a_resources.iter().map(String::as_str).collect();
        let satellites: Vec<&str> = self.satellites.iter().map(String::as_str).collect();
        token_audience(
            &ResourceServers {
                platform: &platform,
                a2a: &a2a,
                satellites: &satellites,
            },
            requested,
            granted,
        )
    }

    /// The delegated grant a live access token this server minted rides on.
    /// `None` for anything else: a token that does not verify or has expired, a
    /// first-party session, a client-credentials token.
    #[must_use]
    pub fn delegated_grant_of_access_token(&self, token: &str) -> Option<TokenGrant> {
        let claims = self
            .auth_manager
            .validate_resource_token(token, &self.jwks_manager, &self.served())
            .ok()?;
        if OAuthScope::is_self_grant(&OAuthScope::parse_granted(&claims.scope))
            || claims.client_credentials_client().is_some()
        {
            return None;
        }
        Some(TokenGrant {
            user_id: Uuid::parse_str(&claims.sub).ok()?,
            tenant_id: claims.active_tenant_id?,
            client_id: claims.client_id?,
            scope: claims.scope,
        })
    }

    /// Refuse a grant whose tenant `user_id` no longer belongs to.
    ///
    /// A grant is authorized in one tenant, and every token it mints acts in
    /// that tenant: the platform's `/mcp` takes the token's
    /// `active_tenant_id`, and a satellite, which holds no tenant data, takes
    /// it on trust. A grant outlives the membership it was authorized under
    /// (its refresh token is good for days), so membership is confirmed at
    /// every mint. Without the tenant there is nothing the athlete consented
    /// to act in, so no token is minted at all.
    ///
    /// # Errors
    /// `invalid_grant` when the membership is gone; `server_error` when it
    /// cannot be read, which never mints a token either.
    pub(super) async fn require_grant_tenant(
        &self,
        user_id: Uuid,
        tenant_id: &str,
    ) -> Result<(), OAuth2Error> {
        if self.still_member(user_id, tenant_id).await? {
            Ok(())
        } else {
            Err(left_tenant(user_id))
        }
    }

    /// Whether `user_id` still belongs to `tenant_id`.
    async fn still_member(&self, user_id: Uuid, tenant_id: &str) -> Result<bool, OAuth2Error> {
        let tenants = self.tenants.list_for_user(user_id).await.map_err(|e| {
            error!(user_id = %user_id, error = %e, "Could not read the user's tenants to mint a token");
            OAuth2Error::server_error("Failed to confirm the grant's tenant")
        })?;
        Ok(tenants
            .iter()
            .any(|tenant| tenant.id.to_string() == tenant_id))
    }

    /// Check a refresh before its token is rotated away: the requested
    /// resource must be the grant's resource server, and the grant's tenant
    /// must still be the user's. A tenant the user left revokes the token's
    /// whole family, so the grant cannot be retried.
    ///
    /// A token that is unknown, spent or expired passes here and is refused by
    /// the rotation, which reads it under the client's id. The grant it read
    /// comes back, `None` when the token is unknown or spent.
    ///
    /// # Errors
    /// `invalid_target` for a resource outside the grant's resource server,
    /// `invalid_grant` for a tenant the user left, `server_error` when the
    /// token, the membership or the revocation cannot be read or written.
    pub(super) async fn check_refresh_grant(
        &self,
        presented: &str,
        requested: Option<&str>,
    ) -> Result<Option<OAuth2RefreshToken>, OAuth2Error> {
        let grant = self
            .oauth2_server
            .get_refresh_token_by_value(presented)
            .await
            .map_err(|e| {
                error!(error = %e, "Could not read the refresh token's grant");
                OAuth2Error::server_error("Failed to read the refresh token")
            })?;
        let Some(grant) = grant.filter(|grant| !grant.revoked) else {
            return Ok(None);
        };
        self.audience(requested, grant.resource.as_deref())?;
        if self.still_member(grant.user_id, &grant.tenant_id).await? {
            return Ok(Some(grant));
        }
        self.oauth2_server
            .revoke_refresh_token_family(presented)
            .await
            .map_err(|e| {
                error!(error = %e, "Could not revoke a grant whose tenant was left");
                OAuth2Error::server_error("Failed to revoke the refresh token")
            })?;
        Err(left_tenant(grant.user_id))
    }

    /// Count a refresh in its user's window, before the token is rotated away.
    ///
    /// Only a grant the rotation would exchange for `client_id` counts, so a
    /// replayed or expired token cannot fill the window of the athlete it
    /// belonged to, nor can another client's request.
    ///
    /// # Errors
    /// As [`Self::admit_grant_user`].
    pub(super) async fn admit_refresh_user(
        &self,
        grant: Option<&OAuth2RefreshToken>,
        client_id: &str,
    ) -> Result<(), OAuth2Error> {
        let live =
            grant.filter(|grant| grant.client_id == client_id && grant.expires_at > Utc::now());
        match live {
            Some(grant) => {
                self.admit_grant_user(grant.user_id, client_id, "refresh_token")
                    .await
            }
            None => Ok(()),
        }
    }

    /// Count a code exchange in its user's window, before the code is spent:
    /// a refusal after the consume would leave the client a code it can never
    /// redeem, and the athlete to authorize again. Hence the extra read, on a
    /// path a client takes once per connection.
    ///
    /// # Errors
    /// As [`Self::admit_grant_user`]; `server_error` when the code cannot be read.
    pub(super) async fn admit_code_user(
        &self,
        code: &str,
        client_id: &str,
        redirect_uri: &str,
    ) -> Result<(), OAuth2Error> {
        if self.token_limiter.is_none() {
            return Ok(());
        }
        let live = self
            .oauth2_server
            .peek_auth_code(code, client_id, redirect_uri, Utc::now())
            .await
            .map_err(|e| {
                error!(client_id, error = %e, "Could not read the authorization code's user");
                OAuth2Error::server_error("Failed to read the authorization code")
            })?;
        match live {
            Some(live) => {
                self.admit_grant_user(live.user_id, client_id, "authorization_code")
                    .await
            }
            None => Ok(()),
        }
    }

    /// Count one redemption in `user_id`'s window, shared by every replica
    /// through the limiter's store.
    ///
    /// # Errors
    /// `too_many_requests` with its wait when the window is full;
    /// `temporarily_unavailable` when the limiter cannot count the redemption,
    /// which leaves the grant unspent for the client's retry.
    async fn admit_grant_user(
        &self,
        user_id: Uuid,
        client_id: &str,
        grant_type: &str,
    ) -> Result<(), OAuth2Error> {
        let Some(limiter) = &self.token_limiter else {
            return Ok(());
        };
        let status = limiter
            .check_account_rate_limit(OAuth2Endpoint::TokenUser, user_id)
            .await
            .map_err(|e| {
                error!(%user_id, client_id, error = %e, "OAuth2 rate limiter could not count a user's token request");
                OAuth2Error::temporarily_unavailable(
                    "Rate limiting is temporarily unavailable; retry later",
                )
            })?;
        if !status.is_limited {
            return Ok(());
        }
        warn!(
            %user_id,
            client_id,
            grant_type,
            retry_after = status.retry_after_seconds,
            limit = status.limit,
            "OAuth token request refused: per-user window full"
        );
        Err(OAuth2Error {
            retry_after: status.retry_after_seconds,
            ..OAuth2Error::too_many_requests("Rate limit exceeded")
        })
    }
}

/// The refusal of a grant whose tenant the user no longer belongs to.
fn left_tenant(user_id: Uuid) -> OAuth2Error {
    warn!(user_id = %user_id, "Grant refused: the user no longer belongs to its tenant");
    OAuth2Error::invalid_grant(
        "The athlete no longer belongs to the tenant this grant was authorized in",
    )
}
