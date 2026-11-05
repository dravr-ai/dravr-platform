// ABOUTME: Rotating a presented OAuth 2.0 refresh token into its successor
// ABOUTME: Consumes the token once, keeps its family and lifetime, and yields to a consent withdrawn meanwhile
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::AppResult;
use secrecy::SecretString;

use super::OAuth2AuthorizationServer;
use crate::oauth2_server::models::OAuth2RefreshToken;
use crate::refresh_rotation::{consume_or_revoke_family, generate_refresh_token, successor_expiry};

impl OAuth2AuthorizationServer {
    /// Exchange a presented refresh token for its successor.
    ///
    /// Follows the one rotation rule in [`crate::refresh_rotation`]: the token
    /// is consumed in the statement that reads it (live, unexpired and owned by
    /// `client_id`), and the successor joins its family. A token that no longer
    /// exchanges revokes its whole family and yields `None`, as does an
    /// unknown one.
    ///
    /// Returns the consumed token's record and the successor's value.
    pub(super) async fn rotate_refresh_token(
        &self,
        presented: &str,
        client_id: &str,
    ) -> AppResult<Option<(OAuth2RefreshToken, String)>> {
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
        // `invalid_grant` and the athlete consents again.
        let Some(expires_at) =
            successor_expiry(now, self.refresh_token_lifetime, consumed.family_created_at)
        else {
            self.oauth2_server
                .revoke_refresh_token_family(presented)
                .await?;
            return Ok(None);
        };

        let successor_value = generate_refresh_token()?;
        let successor = OAuth2RefreshToken {
            token: SecretString::from(successor_value.clone()), // Safe: Clone for storage
            client_id: consumed.client_id.clone(),
            user_id: consumed.user_id,
            tenant_id: consumed.tenant_id.clone(), // Safe: Clone for tenant isolation
            scope: Self::rendered_scope(&Self::delegated_grant(consumed.scope.as_deref())),
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

        Ok(Some((consumed, successor_value)))
    }
}
