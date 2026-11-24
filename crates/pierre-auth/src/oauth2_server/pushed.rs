// ABOUTME: RFC 9126 pushed authorization requests — the request_uri a client is handed and how long it resolves
// ABOUTME: Mints the opaque URI, hashes it for storage, and names the authorization parameters a push may carry
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Pushed authorization requests (RFC 9126).
//!
//! A client sends its whole authorization request to `POST /oauth2/par`,
//! authenticated as at the token endpoint, and is handed a `request_uri` to
//! send the browser with instead. The URI is 32 random bytes; only its
//! SHA-256 is stored, so the table never holds a value that resolves.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use pierre_core::errors::{AppError, AppResult};
use ring::rand::{SecureRandom, SystemRandom};
use sha2::{Digest, Sha256};

/// The URN prefix of every `request_uri` this server issues (RFC 9126 §2.2).
const REQUEST_URI_PREFIX: &str = "urn:ietf:params:oauth:request_uri:";

/// Seconds a pushed request resolves before the browser first brings it to
/// `/oauth2/authorize`: the `expires_in` the push is answered with.
pub const PUSHED_REQUEST_LIFETIME_SECS: i64 = 90;

/// Seconds a pushed request resolves from its first use: long enough for the
/// athlete to sign in, by password or through Google, and consent, each step
/// carrying the `request_uri` and resolving it again.
pub const RESOLVED_REQUEST_LIFETIME_SECS: i64 = 30 * 60;

/// The authorization parameters a push stores, beside `client_id`: what
/// `/oauth2/authorize` reads from a request. Client credentials are never
/// among them.
pub const PUSHED_PARAMETERS: [&str; 10] = [
    "response_type",
    "redirect_uri",
    "scope",
    "state",
    "code_challenge",
    "code_challenge_method",
    "resource",
    "prompt",
    "ui_locales",
    "authorization_details",
];

/// A fresh `request_uri`: the URN prefix and 32 bytes of OS randomness,
/// base64url without padding.
///
/// # Errors
/// Returns an internal error if the system RNG fails.
pub fn new_request_uri() -> AppResult<String> {
    let mut bytes = [0u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| AppError::internal("System RNG failure while minting a request_uri"))?;
    Ok(format!(
        "{REQUEST_URI_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(bytes)
    ))
}

/// The SHA-256 hex a `request_uri` is stored and looked up by.
#[must_use]
pub fn request_uri_hash(request_uri: &str) -> String {
    format!("{:x}", Sha256::digest(request_uri.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_uri_is_the_urn_and_32_random_bytes() -> AppResult<()> {
        let first = new_request_uri()?;
        let second = new_request_uri()?;
        assert_ne!(first, second, "bug: two request_uris repeat");
        let encoded = first.strip_prefix(REQUEST_URI_PREFIX).unwrap_or_default();
        assert_eq!(
            URL_SAFE_NO_PAD.decode(encoded).map(|b| b.len()),
            Ok(32),
            "bug: a request_uri is not 32 bytes after its prefix"
        );
        Ok(())
    }

    #[test]
    fn the_stored_hash_is_the_uri_sha256() {
        assert_eq!(
            request_uri_hash("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            "bug: the stored hash is not the request_uri's SHA-256"
        );
    }
}
