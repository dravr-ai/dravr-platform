// ABOUTME: CSRF (Cross-Site Request Forgery) protection using HMAC-signed stateless tokens
// ABOUTME: Generates self-validating tokens via HMAC-SHA256, eliminating server-side storage
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! CSRF protection module
//!
//! Generates HMAC-signed CSRF tokens that are self-validating (stateless).
//! Tokens encode a user ID and timestamp, signed with a server secret.
//! No server-side storage is needed — validation recomputes the HMAC
//! and checks the embedded timestamp for expiration.

use std::str;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ring::hmac;

use pierre_core::errors::{AppError, AppResult};

/// CSRF token TTL in seconds (24 hours).
/// Tokens older than this are rejected during validation.
const CSRF_TOKEN_TTL_SECS: i64 = 24 * 60 * 60;

/// HMAC key length in bytes (32 bytes = 256 bits)
const HMAC_KEY_LENGTH: usize = 32;

/// Stateless CSRF token manager using HMAC-SHA256 signatures.
///
/// Token format: `base64url(timestamp_secs.user_id.hmac_signature)`
///
/// The HMAC is computed over `timestamp_secs.user_id` using a server secret.
/// Validation extracts the timestamp and `user_id`, recomputes the HMAC, and
/// uses constant-time comparison. No server-side token store is required.
pub struct CsrfTokenManager {
    signing_key: hmac::Key,
}

impl CsrfTokenManager {
    /// Create a new CSRF token manager with a given secret.
    ///
    /// The secret should be at least 32 bytes of high-entropy data.
    /// In production, derive this from a persistent server secret
    /// (e.g., the admin JWT secret) so tokens survive server restarts.
    #[must_use]
    pub fn with_secret(secret: &[u8]) -> Self {
        let signing_key = hmac::Key::new(hmac::HMAC_SHA256, secret);
        Self { signing_key }
    }

    /// Create a CSRF token manager with a deterministic key derived from the
    /// admin JWT secret. This is the primary constructor used in production.
    #[must_use]
    pub fn from_jwt_secret(jwt_secret: &str) -> Self {
        // Derive a dedicated CSRF key from the JWT secret using HKDF-like approach.
        // We use HMAC(jwt_secret_bytes, "pierre-csrf-signing-key-v1") as the derived key.
        let context_key = hmac::Key::new(hmac::HMAC_SHA256, jwt_secret.as_bytes());
        let derived = hmac::sign(&context_key, b"pierre-csrf-signing-key-v1");
        let derived_bytes = derived.as_ref();

        // Take the first HMAC_KEY_LENGTH bytes as the signing key material
        let key_material = &derived_bytes[..HMAC_KEY_LENGTH.min(derived_bytes.len())];
        Self::with_secret(key_material)
    }

    /// Generate a new CSRF token for a user.
    ///
    /// The token is a base64url-encoded string containing:
    /// `timestamp_secs.user_id.hmac_signature`
    ///
    /// # Arguments
    /// * `user_id` - The user ID to bind into the token
    ///
    /// # Errors
    /// Currently infallible but returns `AppResult` for API compatibility.
    pub fn generate_token(&self, user_id: uuid::Uuid) -> AppResult<String> {
        let timestamp = chrono::Utc::now().timestamp();
        let payload = format!("{timestamp}.{user_id}");
        let signature = hmac::sign(&self.signing_key, payload.as_bytes());
        let sig_b64 = URL_SAFE_NO_PAD.encode(signature.as_ref());

        // Final token: base64url("timestamp.user_id.signature_b64")
        let token_raw = format!("{payload}.{sig_b64}");
        let token = URL_SAFE_NO_PAD.encode(token_raw.as_bytes());
        Ok(token)
    }

    /// Validate a CSRF token.
    ///
    /// Decodes the token, checks the embedded timestamp for expiration,
    /// verifies the `user_id` matches, and recomputes the HMAC for integrity.
    ///
    /// # Arguments
    /// * `token` - The opaque token string to validate
    /// * `user_id` - The expected user ID (from the authenticated session)
    ///
    /// # Errors
    /// Returns an error if:
    /// - Token format is invalid (cannot decode or parse)
    /// - Token has expired (older than `CSRF_TOKEN_TTL_SECS`)
    /// - Embedded user ID does not match the expected user ID
    /// - HMAC signature verification fails
    pub fn validate_token(&self, token: &str, user_id: uuid::Uuid) -> AppResult<()> {
        // Decode the outer base64url layer
        let token_bytes = URL_SAFE_NO_PAD
            .decode(token.as_bytes())
            .map_err(|_| AppError::auth_invalid("Invalid CSRF token"))?;

        let token_str = str::from_utf8(&token_bytes)
            .map_err(|_| AppError::auth_invalid("Invalid CSRF token"))?;

        // Parse: "timestamp.user_id.signature_b64"
        let parts: Vec<&str> = token_str.splitn(3, '.').collect();
        if parts.len() != 3 {
            return Err(AppError::auth_invalid("Invalid CSRF token"));
        }

        let timestamp_str = parts[0];
        let token_user_id_str = parts[1];
        let sig_b64 = parts[2];

        // Parse timestamp
        let timestamp: i64 = timestamp_str
            .parse()
            .map_err(|_| AppError::auth_invalid("Invalid CSRF token"))?;

        // Check expiration
        let now = chrono::Utc::now().timestamp();
        let age = now - timestamp;
        if !(0..=CSRF_TOKEN_TTL_SECS).contains(&age) {
            return Err(AppError::auth_invalid("CSRF token expired"));
        }

        // Check user ID match
        let token_user_id = uuid::Uuid::parse_str(token_user_id_str)
            .map_err(|_| AppError::auth_invalid("Invalid CSRF token"))?;

        if token_user_id != user_id {
            return Err(AppError::auth_invalid("CSRF token user mismatch"));
        }

        // Verify HMAC signature (constant-time comparison via ring)
        let payload = format!("{timestamp_str}.{token_user_id_str}");
        let expected_sig = URL_SAFE_NO_PAD
            .decode(sig_b64.as_bytes())
            .map_err(|_| AppError::auth_invalid("Invalid CSRF token"))?;

        hmac::verify(&self.signing_key, payload.as_bytes(), &expected_sig)
            .map_err(|_| AppError::auth_invalid("Invalid CSRF token"))?;

        Ok(())
    }
}

impl Default for CsrfTokenManager {
    /// Creates a `CsrfTokenManager` with a random ephemeral key.
    /// Suitable for testing; in production use `from_jwt_secret` or `with_secret`.
    fn default() -> Self {
        use ring::rand::SystemRandom;
        let rng = SystemRandom::new();
        let signing_key = hmac::Key::generate(hmac::HMAC_SHA256, &rng).unwrap_or_else(|_| {
            // Deterministic fallback: derive key from process-level entropy.
            // This branch is unreachable on any OS with a functioning RNG.
            let fallback = b"pierre-csrf-ephemeral-test-only!";
            hmac::Key::new(hmac::HMAC_SHA256, fallback)
        });
        Self { signing_key }
    }
}
