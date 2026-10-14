// ABOUTME: What a grant must still hold before it mints: its resource server, and its tenant membership
// ABOUTME: A grant bound to one resource server never mints for another; a tenant the user left ends the grant
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use tracing::{error, warn};
use uuid::Uuid;

use super::OAuth2AuthorizationServer;
use crate::oauth2_server::models::OAuth2Error;
use crate::oauth2_server::resource::{token_audience, ResourceServers};

impl OAuth2AuthorizationServer {
    /// The audience of a token minted from a grant bound to `granted`, when
    /// the request names `requested`: [`token_audience`] over the platform's
    /// identifiers and the satellites'.
    ///
    /// # Errors
    /// As [`token_audience`].
    pub(super) fn audience(
        &self,
        requested: Option<&str>,
        granted: Option<&str>,
    ) -> Result<Option<String>, OAuth2Error> {
        let platform: Vec<&str> = self.resources.iter().map(String::as_str).collect();
        let satellites: Vec<&str> = self.satellites.iter().map(String::as_str).collect();
        token_audience(
            &ResourceServers {
                platform: &platform,
                satellites: &satellites,
            },
            requested,
            granted,
        )
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
    /// the rotation, which reads it under the client's id.
    ///
    /// # Errors
    /// `invalid_target` for a resource outside the grant's resource server,
    /// `invalid_grant` for a tenant the user left, `server_error` when the
    /// token, the membership or the revocation cannot be read or written.
    pub(super) async fn check_refresh_grant(
        &self,
        presented: &str,
        requested: Option<&str>,
    ) -> Result<(), OAuth2Error> {
        let grant = self
            .oauth2_server
            .get_refresh_token_by_value(presented)
            .await
            .map_err(|e| {
                error!(error = %e, "Could not read the refresh token's grant");
                OAuth2Error::server_error("Failed to read the refresh token")
            })?;
        let Some(grant) = grant.filter(|grant| !grant.revoked) else {
            return Ok(());
        };
        self.audience(requested, grant.resource.as_deref())?;
        if self.still_member(grant.user_id, &grant.tenant_id).await? {
            return Ok(());
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
}

/// The refusal of a grant whose tenant the user no longer belongs to.
fn left_tenant(user_id: Uuid) -> OAuth2Error {
    warn!(user_id = %user_id, "Grant refused: the user no longer belongs to its tenant");
    OAuth2Error::invalid_grant(
        "The athlete no longer belongs to the tenant this grant was authorized in",
    )
}
