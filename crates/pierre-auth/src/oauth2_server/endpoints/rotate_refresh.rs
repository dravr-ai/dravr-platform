// ABOUTME: Rotating a presented OAuth 2.0 refresh token into its successor
// ABOUTME: Consumes the token once, keeps its family and lifetime, and yields to a consent withdrawn meanwhile; admin lives a day
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::AppResult;
use pierre_core::permissions::scopes::OAuthScope;
use secrecy::SecretString;

use super::admin_grant::holds_admin;
use super::OAuth2AuthorizationServer;
use crate::oauth2_server::models::OAuth2RefreshToken;
use crate::refresh_rotation::{
    admin_successor_expiry, consume_or_revoke_family, generate_refresh_token, successor_expiry,
};

/// A rotated refresh token: the consumed token's record, the successor's
/// value, and the grant both the successor and the access token minted with
/// it carry.
pub(super) type Rotated = (OAuth2RefreshToken, String, Vec<OAuthScope>);

impl OAuth2AuthorizationServer {
    /// Exchange a presented refresh token for its successor.
    ///
    /// Follows the one rotation rule in [`crate::refresh_rotation`]: the token
    /// is consumed in the statement that reads it (live, unexpired and owned by
    /// `client_id`), and the successor joins its family. A token that no longer
    /// exchanges revokes its whole family and yields `None`, as does an
    /// unknown one.
    ///
    /// The grant is decided once here, `admin` read live
    /// ([`Self::delegated_grant_for`]), and both the successor and the access
    /// token minted with it carry it: a grant that loses `admin` loses it for
    /// good. A family holding `admin` stops a day after its first token
    /// (carnet#1511, Q20).
    ///
    /// Returns the consumed token's record, the successor's value and the
    /// grant.
    pub(super) async fn rotate_refresh_token(
        &self,
        presented: &str,
        client_id: &str,
    ) -> AppResult<Option<Rotated>> {
        let now = Utc::now();
        let Some(consumed) = consume_or_revoke_family(
            self.oauth2_server
                .consume_refresh_token(presented, client_id, now),
            || self.oauth2_server.revoke_refresh_token_family(presented),
        )
        .await?
        else {
            return Ok(None);
        };

        // A family past its absolute life is not continued: the client is told
        // `invalid_grant` and the athlete consents again. A family that held
        // `admin` keeps the admin family's day even once `admin` is dropped.
        let deadline = if holds_admin(consumed.scope.as_deref()) {
            admin_successor_expiry(now, self.refresh_token_lifetime, consumed.family_created_at)
        } else {
            successor_expiry(now, self.refresh_token_lifetime, consumed.family_created_at)
        };
        let Some(expires_at) = deadline else {
            self.oauth2_server
                .revoke_refresh_token_family(presented)
                .await?;
            return Ok(None);
        };

        let granted = self
            .delegated_grant_for(
                consumed.user_id,
                consumed.scope.as_deref(),
                consumed.resource.as_deref(),
            )
            .await;
        let successor_value = generate_refresh_token()?;
        let successor = OAuth2RefreshToken {
            token: SecretString::from(successor_value.clone()), // Safe: Clone for storage
            client_id: consumed.client_id.clone(),
            user_id: consumed.user_id,
            tenant_id: consumed.tenant_id.clone(), // Safe: Clone for tenant isolation
            scope: Self::rendered_scope(&granted),
            expires_at,
            created_at: now,
            revoked: false,
            family_id: consumed.family_id.clone(),
            family_created_at: consumed.family_created_at,
            // Rotation keeps the grant's resource binding.
            resource: consumed.resource.clone(),
        };
        self.store_refresh_token(&successor).await?;

        // The athlete may have withdrawn consent while this rotation ran.
        if self
            .oauth2_server
            .revoke_refresh_tokens_of_withdrawn_grant(
                &consumed.user_id.to_string(),
                &consumed.tenant_id,
                &consumed.client_id,
            )
            .await?
        {
            return Ok(None);
        }

        Ok(Some((consumed, successor_value, granted)))
    }
}
