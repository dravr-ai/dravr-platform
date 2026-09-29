// ABOUTME: Server-side Google OpenID Connect sign-in: the account-chooser URL, the code exchange, the ID token check
// ABOUTME: Accepts an ID token only from its own back-channel token call, bound to the sign-in's nonce and client id
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! "Continue with Google" on the hosted authorization-server login page.
//!
//! The authorization server is the relying party: [`GoogleOidcClient::begin`]
//! mints the `state`, `nonce` and PKCE verifier of one sign-in and the URL of
//! Google's account chooser; the callback hands the code back to
//! [`GoogleOidcClient::exchange_code`], which trades it at Google's token
//! endpoint over the back channel, and [`GoogleOidcClient::verify_id_token`]
//! checks the ID token that answer carries.
//!
//! An ID token is never accepted from the browser on this path. Any Google
//! service account can mint a Google-signed token for an arbitrary audience,
//! so a token is trusted only because this server fetched it with its own
//! client secret and a code Google issued for this sign-in, and because it
//! carries this sign-in's nonce.

use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use dravr_tronc::http_client::describe_request_error;
use dravr_tronc::iam::GoogleKeySet;
use jsonwebtoken::{Algorithm, Validation};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::http_client::api_inner_client;
use pierre_core::models::normalize_email;
use ring::rand::{SecureRandom, SystemRandom};
use serde::Deserialize;
use subtle::ConstantTimeEq;
use tracing::debug;

use crate::config::GoogleSignInConfig;
use crate::google_jwt::decode_google_signed;
use crate::http::oauth_client;
use crate::oauth2_client::PkceParams;

/// The `OpenID` Connect scopes the sign-in asks for: an identity, its email
/// and the display name a new account starts with.
pub const GOOGLE_SIGN_IN_SCOPE: &str = "openid email profile";

/// How long one sign-in may take between the account chooser and the callback.
pub const GOOGLE_SIGN_IN_TTL: Duration = Duration::from_secs(600);

/// The provider name a Google identity is recorded under, the one Firebase
/// reports for its own Google sign-ins.
pub const GOOGLE_PROVIDER: &str = "google.com";

/// The issuers Google signs its ID tokens under; it uses both spellings.
const GOOGLE_ISSUERS: [&str; 2] = ["https://accounts.google.com", "accounts.google.com"];

/// Where Google returns the athlete, under the authorization server's issuer.
const GOOGLE_CALLBACK_PATH: &str = "/oauth2/login/google/callback";

/// Random bytes behind each `state` and `nonce`.
const RANDOM_TOKEN_BYTES: usize = 32;

/// The redirect URI Google returns to for an issuer, the one to register on
/// the Google client. A trailing `/` on the issuer is not doubled.
#[must_use]
pub fn google_callback_url(issuer_url: &str) -> String {
    format!("{}{GOOGLE_CALLBACK_PATH}", issuer_url.trim_end_matches('/'))
}

/// One sign-in, begun: the values the callback must see again, and where to
/// send the athlete. Deliberately not `Debug`: every field is a secret of
/// this sign-in.
pub struct GoogleSignInStart {
    /// Returned by Google on the callback; binds it to this sign-in
    pub state: String,
    /// Carried inside the ID token; binds the token to this sign-in
    pub nonce: String,
    /// PKCE verifier the code exchange proves
    pub code_verifier: String,
    /// Google's account chooser, with every parameter of this sign-in
    pub authorization_url: String,
}

/// The Google account an ID token proved.
#[derive(Debug, Clone)]
pub struct GoogleIdentity {
    /// Google's stable account id (`sub`)
    pub subject: String,
    /// The account's email, normalized; Google verified it
    pub email: String,
    /// The account's display name, when it shares one
    pub name: Option<String>,
}

/// The ID token claims the sign-in reads. `iss`, `aud` and `exp` are checked
/// by the decoder.
#[derive(Deserialize)]
struct GoogleIdTokenClaims {
    sub: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    email_verified: Option<bool>,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    azp: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

/// What Google's token endpoint answers; only the fields the sign-in reads.
#[derive(Deserialize)]
struct TokenEndpointAnswer {
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// The Google `OpenID` Connect relying party the hosted login page signs in through.
pub struct GoogleOidcClient {
    config: GoogleSignInConfig,
    /// Google's ID-token signing keys, cached across sign-ins
    keys: GoogleKeySet,
}

impl GoogleOidcClient {
    /// A client for `config`, reading Google's signing keys from its `jwks_url`.
    #[must_use]
    pub fn new(config: GoogleSignInConfig) -> Self {
        let keys = GoogleKeySet::new(config.jwks_url.clone(), api_inner_client().clone());
        Self { config, keys }
    }

    /// Begin a sign-in that Google will return to `redirect_uri`.
    ///
    /// # Errors
    ///
    /// Returns an internal error when the system random source fails.
    pub fn begin(&self, redirect_uri: &str) -> AppResult<GoogleSignInStart> {
        let state = random_token()?;
        let nonce = random_token()?;
        let pkce = PkceParams::generate();
        let query = [
            ("response_type", "code"),
            ("client_id", self.config.client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("scope", GOOGLE_SIGN_IN_SCOPE),
            ("state", state.as_str()),
            ("nonce", nonce.as_str()),
            ("code_challenge", pkce.code_challenge.as_str()),
            ("code_challenge_method", pkce.code_challenge_method.as_str()),
            ("prompt", "select_account"),
        ]
        .iter()
        .map(|(name, value)| format!("{name}={}", urlencoding::encode(value)))
        .collect::<Vec<_>>()
        .join("&");
        let authorization_url = format!("{}?{query}", self.config.authorization_endpoint);
        Ok(GoogleSignInStart {
            state,
            nonce,
            code_verifier: pkce.code_verifier,
            authorization_url,
        })
    }

    /// Exchange the authorization code Google returned for the ID token it
    /// issued, authenticating with the client secret (`client_secret_post`)
    /// and proving the PKCE verifier. `redirect_uri` must be the one the
    /// sign-in began with.
    ///
    /// # Errors
    ///
    /// `auth_invalid` when Google refuses the code (`invalid_grant`: expired,
    /// already used, or not issued to this client); `external_service` when
    /// Google cannot be reached, refuses this client, or answers without an
    /// ID token. No error carries Google's response body.
    pub async fn exchange_code(
        &self,
        code: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> AppResult<String> {
        let form = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", self.config.client_id.as_str()),
            ("client_secret", self.config.client_secret.as_str()),
            ("redirect_uri", redirect_uri),
            ("code_verifier", code_verifier),
        ];
        let response = oauth_client()?
            .post(&self.config.token_endpoint)
            .form(&form)
            .send()
            .await
            .map_err(|e| {
                AppError::external_service(
                    "google",
                    format!("token endpoint unreachable: {}", describe_request_error(e)),
                )
            })?;
        let status = response.status();
        let answer: Option<TokenEndpointAnswer> = response.json().await.ok();

        if !status.is_success() {
            let code = answer
                .and_then(|a| a.error)
                .filter(|e| is_error_code(e))
                .unwrap_or_else(|| "no error code".to_owned());
            if code == "invalid_grant" {
                return Err(AppError::auth_invalid(
                    "Google refused the authorization code",
                ));
            }
            return Err(AppError::external_service(
                "google",
                format!("token endpoint answered HTTP {} ({code})", status.as_u16()),
            ));
        }

        answer.and_then(|a| a.id_token).ok_or_else(|| {
            AppError::external_service("google", "token endpoint answered without an ID token")
        })
    }

    /// Verify the ID token this server received from Google's token endpoint
    /// for the sign-in that minted `expected_nonce`.
    ///
    /// Checks the RS256 signature under Google's published keys, the issuer,
    /// the audience (this client), expiry, and that `iss`, `aud`, `exp` and
    /// `sub` are present; then the nonce, the authorized party when named,
    /// and that Google verified the email it names.
    ///
    /// # Errors
    ///
    /// An authentication error for any token that fails a check; an internal
    /// error when Google's signing keys cannot be fetched.
    pub async fn verify_id_token(
        &self,
        id_token: &str,
        expected_nonce: &str,
    ) -> AppResult<GoogleIdentity> {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[self.config.client_id.as_str()]);
        validation.set_issuer(&GOOGLE_ISSUERS);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);

        let claims: GoogleIdTokenClaims =
            decode_google_signed(&self.keys, id_token, &validation, "Google sign-in").await?;

        let nonce_matches = claims
            .nonce
            .as_deref()
            .is_some_and(|nonce| bool::from(nonce.as_bytes().ct_eq(expected_nonce.as_bytes())));
        if !nonce_matches {
            debug!("Google ID token nonce is missing or belongs to another sign-in");
            return Err(AppError::auth_invalid("Invalid token"));
        }
        if claims
            .azp
            .as_deref()
            .is_some_and(|azp| azp != self.config.client_id)
        {
            debug!("Google ID token was issued to another authorized party");
            return Err(AppError::auth_invalid("Invalid token"));
        }
        let email = claims
            .email
            .as_deref()
            .map(normalize_email)
            .filter(|email| !email.is_empty())
            .ok_or_else(|| AppError::auth_invalid("The Google account shares no email address"))?;
        if claims.email_verified != Some(true) {
            return Err(AppError::auth_invalid(
                "The Google account's email address is not verified",
            ));
        }

        Ok(GoogleIdentity {
            subject: claims.sub,
            email,
            name: claims.name.filter(|name| !name.trim().is_empty()),
        })
    }
}

/// An RFC 6749 error code: lowercase letters and underscores only, so a
/// log line quoting one cannot carry anything else from the response.
fn is_error_code(code: &str) -> bool {
    !code.is_empty() && code.len() <= 64 && code.chars().all(|c| c.is_ascii_lowercase() || c == '_')
}

/// 32 bytes from the system random source, base64url without padding.
fn random_token() -> AppResult<String> {
    let mut bytes = [0u8; RANDOM_TOKEN_BYTES];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| AppError::internal("System random source failed"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
