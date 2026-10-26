// ABOUTME: Signs an athlete straight in after a self-registration that left the account active (carnet#1147)
// ABOUTME: Mints the first-party authorization code the app redeems at /oauth/token, and the register response carrying it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Sign-in at registration.
//!
//! An athlete whose account is created active has just chosen the password,
//! so sending them to the hosted login page to type it again is a step with
//! nothing behind it. The app attaches the authorization request it would
//! have sent `/oauth2/authorize` ([`RegistrationSignIn`]); the registration
//! mints the code that request would have earned, through the same
//! authorization server and its checks (first-party redirect allowlist,
//! mandatory S256 PKCE, state), and the app redeems it at `/oauth/token` with
//! its verifier. No session is set here: the route is CSRF-exempt, and the
//! code is worthless without the verifier the app never sent.
//!
//! A refused sign-in never fails the registration — the account exists — so
//! the response just carries no code and the app falls back to its sign-in
//! screen.

use std::fmt;

use pierre_auth::dto::auth::{RegisterResponse, RegistrationActivation};
use pierre_auth::oauth2_server::first_party::is_first_party;
use pierre_auth::oauth2_server::models::AuthorizeRequest;
use pierre_core::redaction::redact_url;
use serde::Serialize;
use tracing::warn;
use uuid::Uuid;

use crate::signup_guards::RegistrationSignIn;
use crate::token_grant::authorization_server;
use crate::AuthRoutesContext;

/// `POST /api/auth/register`'s answer: the registration, plus the code the
/// app redeems to be signed in when one was minted.
#[derive(Serialize)]
pub(crate) struct PublicRegisterResponse {
    /// The registration, its keys unchanged
    #[serde(flatten)]
    pub registration: RegisterResponse,
    /// The single-use authorization code, present only when the account is
    /// active and the app's sign-in request was accepted
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sign_in_code: Option<String>,
}

/// The code is a credential until it is redeemed, so a `{:?}` of the
/// response prints that one was minted, never the code itself.
impl fmt::Debug for PublicRegisterResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PublicRegisterResponse")
            .field("registration", &self.registration)
            .field(
                "sign_in_code",
                &self.sign_in_code.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// The code that signs the registered athlete in, when the registration
/// earned one: the account came out active (as the registration decided it,
/// never re-read), the app asked, and the app is one of Dravr's own.
pub(crate) async fn registration_sign_in_code(
    resources: &AuthRoutesContext,
    registration: &RegisterResponse,
    sign_in: Option<&RegistrationSignIn>,
) -> Option<String> {
    if registration.activation != RegistrationActivation::Active {
        return None;
    }
    let sign_in = sign_in?;
    if !is_first_party(&sign_in.client_id) {
        warn!(
            client_id = %sign_in.client_id,
            "Registration sign-in refused: not a first-party client"
        );
        return None;
    }
    let Ok(user_id) = Uuid::parse_str(&registration.user_id) else {
        warn!("Registration sign-in skipped: the registered user id is not a UUID");
        return None;
    };
    mint_code(resources, user_id, sign_in).await
}

/// Mint the code through the authorization server the code will be redeemed
/// against, so its every check applies.
async fn mint_code(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    sign_in: &RegistrationSignIn,
) -> Option<String> {
    let request = AuthorizeRequest {
        response_type: "code".to_owned(),
        client_id: sign_in.client_id.clone(),
        redirect_uri: sign_in.redirect_uri.clone(),
        scope: None,
        state: sign_in.state.clone(),
        code_challenge: Some(sign_in.code_challenge.clone()),
        code_challenge_method: sign_in.code_challenge_method.clone(),
        resource: None,
    };
    match authorization_server(resources)
        .authorize(request, Some(user_id), None)
        .await
    {
        Ok(authorized) => Some(authorized.code),
        Err(refusal) => {
            warn!(
                client_id = %sign_in.client_id,
                redirect_uri = %redact_url(&sign_in.redirect_uri),
                error = %refusal.error,
                "Registration sign-in refused; the account stays registered"
            );
            None
        }
    }
}
