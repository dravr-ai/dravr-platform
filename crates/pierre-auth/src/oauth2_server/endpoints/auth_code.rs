// ABOUTME: Minting an authorization code: its PKCE challenge, resource, consent grant and DPoP binding
// ABOUTME: Also stores the state the code is bound to, refusing a state the client already used

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::errors::AppResult;
use tracing::{debug, error, warn};
use uuid::Uuid;

use super::OAuth2AuthorizationServer;
use crate::oauth2_server::models::{OAuth2AuthCode, OAuth2Error, OAuth2State};

/// Parameters for authorization code generation
pub(super) struct AuthCodeParams<'a> {
    pub(super) client_id: &'a str,
    pub(super) user_id: Uuid,
    pub(super) tenant_id: &'a str,
    pub(super) redirect_uri: &'a str,
    pub(super) scope: Option<&'a str>,
    pub(super) state: Option<&'a str>,
    pub(super) code_challenge: Option<&'a str>,
    pub(super) code_challenge_method: Option<&'a str>,
    pub(super) resource: Option<&'a str>,
    /// The consent grant the code is issued under; `None` for a code no
    /// consent grant governs
    pub(super) client_grant_id: Option<&'a str>,
    /// The `DPoP` key thumbprint the code is bound to (RFC 9449 §10)
    pub(super) dpop_jkt: Option<&'a str>,
}

/// Length of an RFC 7638 SHA-256 thumbprint, base64url without padding.
const JKT_LENGTH: usize = 43;

/// Refuse a `dpop_jkt` (RFC 9449 §10) that is no SHA-256 key thumbprint:
/// 43 base64url characters. A code bound to it could never be redeemed.
///
/// # Errors
/// `invalid_request` naming the parameter.
pub(super) fn check_dpop_jkt(dpop_jkt: Option<&str>) -> Result<(), OAuth2Error> {
    let well_formed = |jkt: &str| {
        jkt.len() == JKT_LENGTH
            && jkt
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    };
    match dpop_jkt {
        Some(jkt) if !well_formed(jkt) => Err(OAuth2Error::invalid_request(
            "dpop_jkt must be a base64url SHA-256 JWK thumbprint (RFC 9449 §10)",
        )),
        _ => Ok(()),
    }
}

impl OAuth2AuthorizationServer {
    /// Generate authorization code
    pub(super) async fn generate_authorization_code(
        &self,
        params: AuthCodeParams<'_>,
    ) -> AppResult<String> {
        let code = Self::generate_random_string(32)?;
        let expires_at = Timestamp::now() + Elapsed::minutes(10); // 10 minute expiry

        let auth_code = OAuth2AuthCode {
            code: code.clone(), // Safe: String ownership for OAuth2AuthCode struct
            client_id: params.client_id.to_owned(),
            user_id: params.user_id,
            tenant_id: params.tenant_id.to_owned(),
            redirect_uri: params.redirect_uri.to_owned(),
            scope: params.scope.map(str::to_owned),
            expires_at,
            used: false,
            state: params.state.map(str::to_owned),
            code_challenge: params.code_challenge.map(str::to_owned),
            code_challenge_method: params.code_challenge_method.map(str::to_owned),
            resource: params.resource.map(str::to_owned),
            client_grant_id: params.client_grant_id.map(str::to_owned),
            dpop_jkt: params.dpop_jkt.map(str::to_owned),
        };

        self.store_auth_code(&auth_code).await?;

        // Server-Side State Validation (Defense-in-Depth CSRF Protection)
        //
        // RFC 6749 § 10.12 BASELINE: State is client-side CSRF protection. Server echoes state unchanged.
        // The state parameter is OPAQUE to the server - clients generate it, store it in their session,
        // and validate it matches on callback. Server's only job is to echo it back.
        //
        // OWASP ENHANCEMENT: We ALSO validate state server-side for defense-in-depth security.
        //
        // Why defense-in-depth?
        // 1. Early CSRF Detection: Detects attacks at the server level before client validation
        // 2. Replay Prevention: 10-minute TTL + single-use flag prevents state reuse
        // 3. Client Binding: State bound to client_id prevents cross-client attacks
        // 4. Tenant Isolation: State bound to tenant_id enforces multi-tenant security
        // 5. Audit Trail: Server-side validation provides security event logging
        //
        // Implementation: oauth2_states table (src/database/mod.rs:232)
        // Consumption: validate_and_consume_auth_code() in the parent `endpoints` module
        // Tests: tests/oauth2_state_validation_test.rs (7 security scenarios)
        if let Some(state_value) = params.state {
            let oauth2_state = OAuth2State {
                state: state_value.to_owned(),
                client_id: params.client_id.to_owned(),
                user_id: Some(params.user_id),
                tenant_id: Some(params.tenant_id.to_owned()),
                redirect_uri: params.redirect_uri.to_owned(),
                scope: params.scope.map(str::to_owned),
                code_challenge: params.code_challenge.map(str::to_owned),
                code_challenge_method: params.code_challenge_method.map(str::to_owned),
                created_at: Timestamp::now(),
                expires_at,
                used: false,
            };

            self.store_state(&oauth2_state).await?;
        }

        Ok(code)
    }

    /// Store the state an authorization binds its code to.
    ///
    /// `state` is the client's own value and the table's primary key, so a
    /// repeat is the client reusing one (a reloaded authorize URL): refused at
    /// WARN. The insert failing otherwise is the server's, and pages.
    async fn store_state(&self, state: &OAuth2State) -> AppResult<()> {
        if let Err(e) = self.oauth2_server.store_state(state).await {
            if e.is_server_fault() {
                error!(
                    "Failed to store OAuth2 state for client_id={}: {:#}",
                    state.client_id, e
                );
            } else {
                warn!(client_id = %state.client_id, "OAuth2 authorization refused: state already used");
            }
            return Err(e);
        }
        debug!(
            "Stored OAuth2 state for server-side validation: client_id={}, state_length={}",
            state.client_id,
            state.state.len()
        );
        Ok(())
    }
}
