// ABOUTME: Signing the access token an OAuth grant is redeemed for, under the refresh family the grant started
// ABOUTME: The family rides in the grant_id claim, which an admin's support session is bound to (carnet#1511)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{Duration, Utc};
use jsonwebtoken::{encode, Algorithm, Header};
use pierre_core::constants::{
    limits::OAUTH_ACCESS_TOKEN_EXPIRY_HOURS,
    service_names::{MCP, PIERRE_MCP_SERVER},
};
use pierre_core::errors::{AppError, AppResult};
use uuid::Uuid;

use super::{AuthManager, Claims, DelegatedGrant};
use crate::admin::jwks::JwksManager;

impl AuthManager {
    /// Generate OAuth access token with RS256 asymmetric signing
    ///
    /// This method uses RSA private key from JWKS manager for token signing.
    /// Clients can verify tokens using the public key from /.well-known/jwks.json
    ///
    /// `grant` is what the authorization server decided: the registered
    /// client the token is issued to, which it carries in
    /// [`Claims::client_id`], its scope, and its audience.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - JWT token generation fails
    /// - System time is unavailable
    /// - JWKS manager has no active key
    pub fn generate_oauth_access_token(
        &self,
        jwks_manager: &JwksManager,
        grant: &DelegatedGrant<'_>,
        user_id: &Uuid,
        providers: &[String],
        active_tenant_id: Option<String>,
    ) -> AppResult<String> {
        Self::mint_oauth_access_token(
            jwks_manager,
            grant,
            None,
            user_id,
            providers,
            active_tenant_id,
        )
    }

    /// [`Self::generate_oauth_access_token`] for a token minted under the
    /// refresh family `grant_id`, which it carries in [`Claims::grant_id`]:
    /// what the authorization server's code and refresh grants mint.
    ///
    /// # Errors
    ///
    /// As [`Self::generate_oauth_access_token`].
    pub fn generate_oauth_access_token_in_grant(
        &self,
        jwks_manager: &JwksManager,
        grant: &DelegatedGrant<'_>,
        grant_id: &str,
        user_id: &Uuid,
        providers: &[String],
        active_tenant_id: Option<String>,
    ) -> AppResult<String> {
        Self::mint_oauth_access_token(
            jwks_manager,
            grant,
            Some(grant_id),
            user_id,
            providers,
            active_tenant_id,
        )
    }

    /// Sign a delegated access token, under `grant_id` when it has one.
    fn mint_oauth_access_token(
        jwks_manager: &JwksManager,
        grant: &DelegatedGrant<'_>,
        grant_id: Option<&str>,
        user_id: &Uuid,
        providers: &[String],
        active_tenant_id: Option<String>,
    ) -> AppResult<String> {
        let now = Utc::now();
        let expiry = now + Duration::hours(OAUTH_ACCESS_TOKEN_EXPIRY_HOURS);

        let claims = Claims {
            sub: user_id.to_string(),
            email: format!("oauth_{user_id}@system.local"),
            iat: now.timestamp(),
            exp: expiry.timestamp(),
            iss: PIERRE_MCP_SERVER.to_owned(),
            jti: Uuid::new_v4().to_string(),
            // The athlete's connected fitness providers — what this field
            // means everywhere else it is written. It used to receive `scopes`
            // here, so a token minted at the token endpoint reported
            // `fitness:read` as a connected provider; the grant now rides in
            // `scope`, where a resource server can read it.
            providers: providers.to_vec(),
            scope: grant.scopes.join(" "),
            aud: grant.audience.unwrap_or(MCP).to_owned(),
            active_tenant_id,
            impersonator_id: None,
            impersonation_session_id: None,
            client_id: Some(grant.client_id.to_owned()),
            grant_id: grant_id.map(str::to_owned),
        };

        // Get active RSA key from JWKS manager
        let active_key = jwks_manager.get_active_key()?;
        let encoding_key = active_key.encoding_key()?;

        // Create RS256 header with kid
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(active_key.kid.clone());

        let token = encode(&header, &claims, &encoding_key)
            .map_err(|e| AppError::internal(format!("Failed to encode JWT token: {e}")))?;

        Ok(token)
    }
}
