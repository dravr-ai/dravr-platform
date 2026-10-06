// ABOUTME: Redeeming an authorization code issued to one of Dravr's own apps
// ABOUTME: A public client: no secret, the PKCE verifier is mandatory, and the caller mints the session
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use tracing::warn;
use uuid::Uuid;

use super::OAuth2AuthorizationServer;
use crate::oauth2_server::first_party::is_first_party;
use crate::oauth2_server::models::OAuth2Error;

/// Who a redeemed first-party code signs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedeemedFirstPartyCode {
    /// The athlete who signed in on the hosted login page
    pub user_id: Uuid,
    /// The tenant the code was minted for
    pub tenant_id: String,
}

impl OAuth2AuthorizationServer {
    /// Redeem an authorization code issued to Dravr's web or mobile app.
    ///
    /// The code is consumed exactly as a third-party client's is (client,
    /// `redirect_uri`, expiry, single use, `state`), but the client is public:
    /// it proves itself with the PKCE verifier alone, so the verifier is
    /// required rather than checked when present. What the code is worth is
    /// the caller's to mint — a first-party session, never a delegated grant.
    ///
    /// # Errors
    /// `invalid_client` for any other client, `invalid_request` without a
    /// verifier, `invalid_grant` for a code that is unknown, spent, expired,
    /// bound elsewhere or fails PKCE, and `server_error` when it cannot be read.
    pub async fn redeem_first_party_code(
        &self,
        client_id: &str,
        code: &str,
        redirect_uri: &str,
        code_verifier: Option<&str>,
    ) -> Result<RedeemedFirstPartyCode, OAuth2Error> {
        if !is_first_party(client_id) {
            warn!(
                client_id,
                "First-party code exchange refused: not a first-party client"
            );
            return Err(OAuth2Error::invalid_client());
        }
        let Some(verifier) = code_verifier else {
            return Err(OAuth2Error::invalid_request(
                "code_verifier is required: Dravr's apps are public clients (RFC 7636)",
            ));
        };
        let auth_code = self
            .validate_and_consume_auth_code(code, client_id, redirect_uri, Some(verifier))
            .await?;
        Ok(RedeemedFirstPartyCode {
            user_id: auth_code.user_id,
            tenant_id: auth_code.tenant_id,
        })
    }
}
