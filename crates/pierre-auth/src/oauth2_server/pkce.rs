// ABOUTME: PKCE (RFC 7636) checks of the OAuth 2.0 authorization server
// ABOUTME: The code_challenge an authorization request carries, and the code_verifier a token exchange proves it with
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use base64::{engine::general_purpose, Engine as _};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tracing::{debug, warn};

use super::models::{AuthorizeRequest, OAuth2Error};

/// Validate the PKCE parameters of an authorization request (RFC 7636).
pub(super) fn check_code_challenge(request: &AuthorizeRequest) -> Result<(), OAuth2Error> {
    let Some(code_challenge) = &request.code_challenge else {
        // PKCE is required for authorization code flow
        return Err(OAuth2Error::invalid_request(
            "code_challenge is required for authorization_code flow (PKCE)",
        ));
    };

    // Validate code_challenge format (base64url-encoded, 43-128 characters)
    if code_challenge.len() < 43 || code_challenge.len() > 128 {
        return Err(OAuth2Error::invalid_request(
            "code_challenge must be between 43 and 128 characters",
        ));
    }

    // Validate code_challenge_method - only S256 is allowed (RFC 7636 security best practice)
    let method = request.code_challenge_method.as_deref().unwrap_or("S256");
    if method != "S256" {
        return Err(OAuth2Error::invalid_request(
            "code_challenge_method must be 'S256' (plain method is not supported for security reasons)",
        ));
    }
    Ok(())
}

/// Validate PKCE `code_verifier` format per RFC 7636 Section 4.1
fn validate_verifier_format(verifier: &str) -> Result<(), OAuth2Error> {
    // Length: 43-128 characters
    if verifier.len() < 43 || verifier.len() > 128 {
        return Err(OAuth2Error::invalid_grant(
            "code_verifier must be between 43 and 128 characters",
        ));
    }

    // Characters: Only unreserved characters allowed: [A-Z] / [a-z] / [0-9] / "-" / "." / "_" / "~"
    verifier
        .chars()
        .all(|c| matches!(c, 'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '.' | '_' | '~'))
        .ok_or_else(|| {
            OAuth2Error::invalid_grant(
                "code_verifier contains invalid characters (RFC 7636: only [A-Z], [a-z], [0-9], -, ., _, ~ allowed)",
            )
        })
}

/// Compute PKCE challenge from verifier using S256 method
fn compute_challenge(verifier: &str, method: &str) -> Result<String, OAuth2Error> {
    if method != "S256" {
        return Err(OAuth2Error::invalid_grant(
            "Only S256 code_challenge_method is supported (plain method is not allowed for security reasons)",
        ));
    }

    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    Ok(general_purpose::URL_SAFE_NO_PAD.encode(hash))
}

/// Verify PKCE challenge using constant-time comparison
pub(super) fn verify_challenge(
    stored_challenge: &str,
    code_verifier: Option<&str>,
    code_challenge_method: Option<&str>,
    client_id: &str,
) -> Result<(), OAuth2Error> {
    let verifier = code_verifier
        .ok_or_else(|| OAuth2Error::invalid_grant("code_verifier is required (PKCE)"))?;

    validate_verifier_format(verifier)?;

    let method = code_challenge_method.unwrap_or("S256");
    let computed_challenge = compute_challenge(verifier, method)?;

    // Constant-time comparison to prevent timing attacks
    if computed_challenge
        .as_bytes()
        .ct_eq(stored_challenge.as_bytes())
        .into()
    {
        debug!("PKCE verification successful for client {}", client_id);
        Ok(())
    } else {
        warn!(
            "PKCE verification failed for client {} - code_verifier does not match code_challenge",
            client_id
        );
        Err(OAuth2Error::invalid_grant("Invalid code_verifier"))
    }
}
