// ABOUTME: Firebase Authentication token validation module
// ABOUTME: Validates Firebase ID tokens using Google's public keys with automatic key caching
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Firebase Authentication Token Validation
//!
//! This module provides:
//! - Firebase ID token validation using Google's public keys
//! - Token claims extraction (email, provider, etc.)
//!
//! Key fetching and caching is dravr-tronc's [`GoogleKeySet`] pointed at the
//! Firebase `securetoken@system` JWK set; the Firebase claim checks are this
//! module's own.
//!
//! ## Security Model
//!
//! - Public keys fetched from Google's official JWK set endpoint
//! - Keys reused for an hour; an unknown `kid` refetches, but never within
//!   [`dravr_tronc::iam::MIN_REFETCH_INTERVAL`] of the last fetch, so made-up
//!   key ids cannot turn each request into a fetch from Google
//! - Tokens validated for issuer, audience, and expiry
//! - Provider ID extracted from `firebase.sign_in_provider` claim
//!
//! ## Usage
//!
//! ```rust,no_run
//! use pierre_auth::firebase::FirebaseAuth;
//! use pierre_auth::config::oauth::FirebaseConfig;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = FirebaseConfig {
//!     project_id: Some("my-project".to_string()),
//!     api_key: None,
//!     enabled: true,
//! };
//! let firebase = FirebaseAuth::new(config);
//!
//! // Validate a Firebase ID token
//! let claims = firebase.validate_token("eyJ...").await?;
//! println!("User email: {}", claims.email.unwrap_or_default());
//! println!("Provider: {}", claims.provider);
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;

use dravr_tronc::iam::{GoogleKeySet, IamError};
use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, info, warn};

use crate::config::oauth::FirebaseConfig;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::http_client::api_inner_client;

/// Google's JWK set for the keys that sign Firebase Authentication ID tokens
pub const FIREBASE_JWKS_URL: &str =
    "https://www.googleapis.com/service_accounts/v1/jwk/securetoken@system.gserviceaccount.com";

/// Firebase issuer URL template (includes project ID)
const FIREBASE_ISSUER_TEMPLATE: &str = "https://securetoken.google.com/";

/// Firebase ID token claims
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirebaseClaims {
    /// Issuer (should be `https://securetoken.google.com/<project-id>`)
    pub iss: String,
    /// Audience (should be the Firebase project ID)
    pub aud: String,
    /// Subject (Firebase user UID)
    pub sub: String,
    /// Issued at timestamp
    pub iat: i64,
    /// Expiration timestamp
    pub exp: i64,
    /// User email (if available)
    pub email: Option<String>,
    /// Whether email is verified
    pub email_verified: Option<bool>,
    /// User display name (if available)
    pub name: Option<String>,
    /// User profile picture URL (if available)
    pub picture: Option<String>,
    /// Firebase-specific claims
    #[serde(default)]
    pub firebase: FirebaseSpecificClaims,
    /// Authentication provider extracted from `firebase.sign_in_provider`
    #[serde(skip)]
    pub provider: String,
}

/// Firebase-specific claims within the token
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FirebaseSpecificClaims {
    /// Sign-in provider (e.g., "google.com", "apple.com", "password")
    pub sign_in_provider: Option<String>,
    /// Identity claims from the provider
    pub identities: Option<HashMap<String, Value>>,
}

/// Firebase Authentication handler
///
/// Provides token validation with automatic key caching through the
/// Firebase JWK set.
pub struct FirebaseAuth {
    /// Firebase configuration
    config: FirebaseConfig,
    /// Public keys behind the Firebase `securetoken@system` JWK set
    keys: GoogleKeySet,
}

impl FirebaseAuth {
    /// Create a Firebase authentication handler reading Google's published
    /// Firebase signing keys at [`FIREBASE_JWKS_URL`].
    #[must_use]
    pub fn new(config: FirebaseConfig) -> Self {
        Self::with_key_set(
            config,
            GoogleKeySet::new(FIREBASE_JWKS_URL, api_inner_client().clone()),
        )
    }

    /// Create a Firebase authentication handler reading its signing keys from
    /// `keys` — the key set a test serves from a local listener.
    #[must_use]
    pub const fn with_key_set(config: FirebaseConfig, keys: GoogleKeySet) -> Self {
        Self { config, keys }
    }

    /// Check if Firebase authentication is enabled and configured
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.config.is_configured()
    }

    /// Get the Firebase project ID
    #[must_use]
    pub fn project_id(&self) -> Option<&str> {
        self.config.project_id.as_deref()
    }

    /// Validate a Firebase ID token
    ///
    /// # Arguments
    ///
    /// * `token` - The Firebase ID token to validate
    ///
    /// # Returns
    ///
    /// * `Ok(FirebaseClaims)` - The validated token claims
    /// * `Err(AppError)` - If validation fails
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Firebase is not configured
    /// - Token header cannot be decoded
    /// - Public key cannot be found for the token's key ID
    /// - Token signature is invalid
    /// - Token is expired or not yet valid
    /// - Issuer or audience doesn't match
    pub async fn validate_token(&self, token: &str) -> AppResult<FirebaseClaims> {
        // Check if Firebase is configured
        let project_id =
            self.config.project_id.as_ref().ok_or_else(|| {
                AppError::invalid_input("Firebase authentication is not configured")
            })?;

        if !self.config.enabled {
            return Err(AppError::invalid_input(
                "Firebase authentication is disabled",
            ));
        }

        // Decode the token header to get the key ID
        let header = decode_header(token).map_err(|e| {
            debug!(error = %e, "Failed to decode Firebase token header");
            AppError::auth_invalid("Invalid token format")
        })?;

        let kid = header.kid.ok_or_else(|| {
            debug!("Firebase token missing key ID (kid) in header");
            AppError::auth_invalid("Token missing key ID")
        })?;

        // Get the public key for this key ID
        let key = self.keys.key(&kid).await.map_err(|e| match e {
            IamError::Rejected(why) => {
                debug!(kid = %kid, reason = %why, "Firebase token names no published signing key");
                AppError::auth_invalid("Unknown token signing key")
            }
            other => {
                warn!(error = %other, "Firebase signing keys are unavailable");
                AppError::internal("Firebase signing keys are unavailable")
            }
        })?;

        // Create the decoding key from the published modulus and exponent
        let decoding_key = DecodingKey::from_rsa_components(key.modulus(), key.exponent())
            .map_err(|e| {
                warn!(error = %e, kid = %kid, "Firebase signing key did not yield a decoding key");
                AppError::internal(format!("Invalid public key: {e}"))
            })?;

        // Set up validation
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[project_id]);
        validation.set_issuer(&[format!("{FIREBASE_ISSUER_TEMPLATE}{project_id}")]);

        // Decode and validate the token
        let token_data =
            decode::<FirebaseClaims>(token, &decoding_key, &validation).map_err(|e| {
                debug!(error = %e, "Firebase token validation failed");
                match e.kind() {
                    ErrorKind::ExpiredSignature => AppError::auth_expired(),
                    ErrorKind::InvalidAudience => AppError::auth_invalid("Invalid token audience"),
                    ErrorKind::InvalidIssuer => AppError::auth_invalid("Invalid token issuer"),
                    _ => AppError::auth_invalid("Invalid token"),
                }
            })?;

        // Extract the provider from firebase.sign_in_provider
        let mut claims = token_data.claims;
        claims.provider = claims
            .firebase
            .sign_in_provider
            .clone()
            .unwrap_or_else(|| "unknown".to_owned());

        info!(
            user_id = %claims.sub,
            provider = %claims.provider,
            "Firebase token validated successfully"
        );
        debug!(
            user_id = %claims.sub,
            email = claims.email.as_deref().unwrap_or("(none)"),
            "Firebase token claims detail"
        );

        Ok(claims)
    }
}
