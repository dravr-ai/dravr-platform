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
//!     ..FirebaseConfig::default()
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

use dravr_tronc::iam::GoogleKeySet;
use jsonwebtoken::{Algorithm, Validation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, info};

use crate::config::oauth::FirebaseConfig;
use crate::google_jwt::decode_google_signed;
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

impl FirebaseClaims {
    /// The Google account id (Google's own `OpenID` Connect `sub`) behind this
    /// Firebase user, when it signed in with Google: the first entry Firebase
    /// lists under `firebase.identities["google.com"]`. It is not the
    /// Firebase UID in [`Self::sub`], which Firebase mints itself.
    #[must_use]
    pub fn google_subject(&self) -> Option<&str> {
        self.firebase
            .identities
            .as_ref()?
            .get("google.com")?
            .as_array()?
            .first()?
            .as_str()
    }
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

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[project_id]);
        validation.set_issuer(&[format!("{FIREBASE_ISSUER_TEMPLATE}{project_id}")]);

        let mut claims: FirebaseClaims =
            decode_google_signed(&self.keys, token, &validation, "Firebase").await?;

        // Extract the provider from firebase.sign_in_provider
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
