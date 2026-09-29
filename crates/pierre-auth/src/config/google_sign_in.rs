// ABOUTME: The Google OAuth web client the hosted authorization server signs athletes in through
// ABOUTME: Read from GOOGLE_OAUTH_CLIENT_ID/SECRET; its Debug names the client and fingerprints the secret, never prints it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;
use std::fmt;

use dravr_tronc::iam::GOOGLE_OIDC_JWKS_URL;

use super::oauth::secret_fingerprint;

/// Google's `OpenID` Connect authorization endpoint (its account chooser).
pub const GOOGLE_AUTHORIZATION_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";

/// Google's OAuth 2.0 token endpoint, where an authorization code is
/// exchanged for the ID token naming the athlete.
pub const GOOGLE_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";

/// The Google OAuth web client behind "Continue with Google" on the hosted
/// `/oauth2/login` page.
///
/// The endpoints default to Google's own; a test points them at a local stand-in.
#[derive(Clone)]
pub struct GoogleSignInConfig {
    /// OAuth client id of the Google web client (also the ID token audience)
    pub client_id: String,
    /// Its client secret, sent only to the token endpoint
    pub client_secret: String,
    /// Where the athlete is sent to choose a Google account
    pub authorization_endpoint: String,
    /// Where the authorization code is exchanged
    pub token_endpoint: String,
    /// The JWK set Google signs its ID tokens with
    pub jwks_url: String,
}

impl GoogleSignInConfig {
    /// A client over Google's own endpoints.
    #[must_use]
    pub fn new(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            authorization_endpoint: GOOGLE_AUTHORIZATION_ENDPOINT.to_owned(),
            token_endpoint: GOOGLE_TOKEN_ENDPOINT.to_owned(),
            jwks_url: GOOGLE_OIDC_JWKS_URL.to_owned(),
        }
    }

    /// The client `GOOGLE_OAUTH_CLIENT_ID` and `GOOGLE_OAUTH_CLIENT_SECRET`
    /// name, or `None` unless both are set to something other than blanks.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        Self::from_values(
            env::var("GOOGLE_OAUTH_CLIENT_ID").ok().as_deref(),
            env::var("GOOGLE_OAUTH_CLIENT_SECRET").ok().as_deref(),
        )
    }

    /// The client these two values name, trimmed, or `None` unless both are
    /// present and non-blank: a half-configured client could only fail at
    /// Google, after the athlete has already chosen an account.
    #[must_use]
    pub fn from_values(client_id: Option<&str>, client_secret: Option<&str>) -> Option<Self> {
        let client_id = client_id.map(str::trim).filter(|v| !v.is_empty())?;
        let client_secret = client_secret.map(str::trim).filter(|v| !v.is_empty())?;
        Some(Self::new(client_id, client_secret))
    }

    /// The first 8 hex characters of the secret's SHA-256, for telling
    /// deployed secrets apart in a log line.
    #[must_use]
    pub fn secret_fingerprint(&self) -> String {
        secret_fingerprint(&self.client_secret)
    }
}

impl fmt::Debug for GoogleSignInConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GoogleSignInConfig")
            .field("client_id", &self.client_id)
            .field("secret_length", &self.client_secret.len())
            .field("secret_fingerprint", &self.secret_fingerprint())
            .field("authorization_endpoint", &self.authorization_endpoint)
            .field("token_endpoint", &self.token_endpoint)
            .field("jwks_url", &self.jwks_url)
            .finish()
    }
}
