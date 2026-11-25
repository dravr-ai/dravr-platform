// ABOUTME: Signing the access token an OAuth grant is redeemed for, under the refresh family the grant started
// ABOUTME: The family rides in the grant_id claim (support sessions, carnet#1511), the consent grant in client_grant_id

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::{Elapsed, Timestamp};
use jsonwebtoken::{encode, Algorithm, Header};
use pierre_core::constants::limits::{
    OAUTH_ACCESS_TOKEN_EXPIRY_HOURS, OAUTH_DELEGATED_ACCESS_TOKEN_EXPIRY_MINUTES,
};
use pierre_core::constants::service_names::MCP;
use pierre_core::errors::{AppError, AppResult};
use uuid::Uuid;

use super::{AuthManager, Claims, Confirmation, DelegatedGrant};
use crate::admin::jwks::JwksManager;
use crate::oauth2_server::first_party::names_third_party;

/// Where a delegated access token comes from: the refresh family its
/// authorization started, and the consent grant that authorization was given
/// under.
#[derive(Debug, Clone, Copy)]
pub struct TokenLineage<'grant> {
    /// The refresh family ([`Claims::grant_id`])
    pub family_id: &'grant str,
    /// The consent grant ([`Claims::client_grant_id`]); `None` for a family
    /// begun before grants were named
    pub client_grant_id: Option<&'grant str>,
    /// The RFC 7638 thumbprint of the `DPoP` key the token is bound to, which
    /// it carries as `cnf.jkt` ([`Claims::cnf`]); `None` mints a Bearer token
    pub dpop_jkt: Option<&'grant str>,
}

/// How long an access token minted for `client_id` lives, in seconds.
///
/// Fifteen minutes for a third-party client, an hour for Dravr's own apps.
/// The token's `exp`, the token response's `expires_in` and introspection all
/// follow it.
#[must_use]
pub fn oauth_access_token_lifetime_secs(client_id: &str) -> i64 {
    oauth_access_token_lifetime(client_id).num_seconds()
}

/// [`oauth_access_token_lifetime_secs`] as a duration.
fn oauth_access_token_lifetime(client_id: &str) -> Elapsed {
    if names_third_party(Some(client_id)) {
        Elapsed::minutes(OAUTH_DELEGATED_ACCESS_TOKEN_EXPIRY_MINUTES)
    } else {
        Elapsed::hours(OAUTH_ACCESS_TOKEN_EXPIRY_HOURS)
    }
}

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
        active_tenant_id: Option<String>,
    ) -> AppResult<String> {
        self.mint_oauth_access_token(jwks_manager, grant, None, user_id, active_tenant_id)
    }

    /// [`Self::generate_oauth_access_token`] for a token minted under
    /// `lineage`: the refresh family it carries in [`Claims::grant_id`] and
    /// the consent grant it carries in [`Claims::client_grant_id`], what the
    /// authorization server's code and refresh grants mint.
    ///
    /// # Errors
    ///
    /// As [`Self::generate_oauth_access_token`].
    pub fn generate_oauth_access_token_in_grant(
        &self,
        jwks_manager: &JwksManager,
        grant: &DelegatedGrant<'_>,
        lineage: &TokenLineage<'_>,
        user_id: &Uuid,
        active_tenant_id: Option<String>,
    ) -> AppResult<String> {
        self.mint_oauth_access_token(
            jwks_manager,
            grant,
            Some(lineage),
            user_id,
            active_tenant_id,
        )
    }

    /// Sign a delegated access token, under `lineage` when it has one.
    fn mint_oauth_access_token(
        &self,
        jwks_manager: &JwksManager,
        grant: &DelegatedGrant<'_>,
        lineage: Option<&TokenLineage<'_>>,
        user_id: &Uuid,
        active_tenant_id: Option<String>,
    ) -> AppResult<String> {
        let now = Timestamp::now();
        let expiry = now + oauth_access_token_lifetime(grant.client_id);

        let claims = Claims {
            sub: user_id.to_string(),
            email: format!("oauth_{user_id}@system.local"),
            iat: now.epoch_seconds(),
            exp: expiry.epoch_seconds(),
            // The authorization server's issuer (RFC 9068 §2.2), the
            // metadata's `issuer`; sessions keep the platform's own name.
            iss: self.oauth_token_issuer().to_owned(),
            jti: Uuid::new_v4().to_string(),
            scope: grant.scopes.join(" "),
            aud: grant.audience.unwrap_or(MCP).to_owned(),
            active_tenant_id,
            impersonator_id: None,
            impersonation_session_id: None,
            client_id: Some(grant.client_id.to_owned()),
            grant_id: lineage.map(|lineage| lineage.family_id.to_owned()),
            client_grant_id: lineage
                .and_then(|lineage| lineage.client_grant_id)
                .map(str::to_owned),
            cnf: lineage
                .and_then(|lineage| lineage.dpop_jkt)
                .map(|jkt| Confirmation {
                    jkt: jkt.to_owned(),
                }),
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
