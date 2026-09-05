// ABOUTME: Verifies the Google-signed OIDC ID token a Cloud Tasks task carries to an internal route
// ABOUTME: RS256 against the accounts.google.com certificate cache; audience and service account must both match
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The gate on the turn-run route: the OIDC token a Cloud Tasks task carries.
//!
//! The request a Cloud Tasks task delivers is authenticated by the OIDC token
//! Cloud Tasks mints for it — signed by Google, issued for the audience the
//! task named, on behalf of the service account the task named. The backend
//! runs with invoker IAM disabled (its ingress is internal), so this
//! verification is the whole gate: a request without a valid token for
//! exactly this audience and exactly this service account is refused before
//! anything is read from the database.

use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use pierre_core::errors::{AppError, AppResult};
use serde::Deserialize;
use tracing::{debug, warn};

use crate::google_certs::GoogleCertCache;

/// Where Google publishes the certificates that sign its OIDC ID tokens: the
/// `kid` → X.509 PEM map the certificate cache reads.
pub const GOOGLE_OIDC_CERTS_URL: &str = "https://www.googleapis.com/oauth2/v1/certs";

/// The two spellings Google uses for the issuer of its ID tokens.
const GOOGLE_ISSUERS: [&str; 2] = ["https://accounts.google.com", "accounts.google.com"];

/// The claims of a verified Google ID token.
#[derive(Debug, Clone, Deserialize)]
pub struct GoogleIdClaims {
    /// Issuer, one of [`GOOGLE_ISSUERS`].
    pub iss: String,
    /// Audience the token was minted for.
    pub aud: String,
    /// Stable id of the identity the token was minted on behalf of.
    pub sub: String,
    /// Email of that identity — the service account, for a Cloud Tasks token.
    pub email: Option<String>,
    /// Whether Google vouches for `email`.
    pub email_verified: Option<bool>,
    /// Expiry, seconds since the epoch.
    pub exp: u64,
}

/// Verifies ID tokens minted for one audience on behalf of one service account.
pub struct GoogleIdTokenVerifier {
    certs: GoogleCertCache,
    audience: String,
    service_account: String,
}

impl GoogleIdTokenVerifier {
    /// A verifier for tokens minted for `audience` on behalf of
    /// `service_account`, reading Google's published certificates.
    #[must_use]
    pub fn new(audience: impl Into<String>, service_account: impl Into<String>) -> Self {
        Self::with_certs_url(audience, service_account, GOOGLE_OIDC_CERTS_URL)
    }

    /// The same verifier reading its certificates from `certs_url`. Test seam:
    /// a test serves the certificate of its own signing key from a local
    /// listener and mints tokens with that key.
    #[must_use]
    pub fn with_certs_url(
        audience: impl Into<String>,
        service_account: impl Into<String>,
        certs_url: impl Into<String>,
    ) -> Self {
        Self {
            certs: GoogleCertCache::new(certs_url),
            audience: audience.into(),
            service_account: service_account.into(),
        }
    }

    /// The audience every accepted token must carry.
    #[must_use]
    pub fn audience(&self) -> &str {
        &self.audience
    }

    /// The service account every accepted token must be minted on behalf of.
    #[must_use]
    pub fn service_account(&self) -> &str {
        &self.service_account
    }

    /// Verify `token` and return its claims.
    ///
    /// # Errors
    ///
    /// Returns `AppError::auth_invalid` (401) when the token is malformed,
    /// signed by a key Google does not publish, issued by anyone but Google,
    /// minted for another audience, expired, or minted on behalf of any
    /// identity but the configured service account. Every refusal is logged
    /// at WARN with its reason; the error carries none of the token.
    pub async fn verify(&self, token: &str) -> AppResult<GoogleIdClaims> {
        let header = decode_header(token).map_err(|e| {
            warn!(error = %e, "Google ID token header did not decode");
            AppError::auth_invalid("Invalid token format")
        })?;
        let kid = header.kid.ok_or_else(|| {
            warn!("Google ID token carries no key id");
            AppError::auth_invalid("Token missing key ID")
        })?;

        let pem_key = self.certs.public_key(&kid).await?;
        let decoding_key = DecodingKey::from_rsa_pem(pem_key.as_bytes()).map_err(|e| {
            warn!(error = %e, kid = %kid, "Google certificate did not yield a decoding key");
            AppError::internal(format!("Invalid public key: {e}"))
        })?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[self.audience.as_str()]);
        validation.set_issuer(&GOOGLE_ISSUERS);

        let claims = decode::<GoogleIdClaims>(token, &decoding_key, &validation)
            .map_err(|e| {
                warn!(error = %e, "Google ID token refused");
                match e.kind() {
                    ErrorKind::ExpiredSignature => AppError::auth_expired(),
                    ErrorKind::InvalidAudience => AppError::auth_invalid("Invalid token audience"),
                    ErrorKind::InvalidIssuer => AppError::auth_invalid("Invalid token issuer"),
                    _ => AppError::auth_invalid("Invalid token"),
                }
            })?
            .claims;

        let minted_for = claims.email.as_deref().unwrap_or_default();
        if minted_for != self.service_account || claims.email_verified != Some(true) {
            warn!(
                sub = %claims.sub,
                verified = claims.email_verified.unwrap_or(false),
                "Google ID token was minted for an identity other than the turn runner"
            );
            return Err(AppError::auth_invalid(
                "Token subject is not the turn runner",
            ));
        }

        debug!(sub = %claims.sub, aud = %claims.aud, "Google ID token verified");
        Ok(claims)
    }
}
