// ABOUTME: Selector/verifier link-token generation + parsing (T3MP3ST F1)
// ABOUTME: Delivered token is "<selector>.<verifier>"; only the verifier's SHA-256 hash is stored
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Emailed link-token helpers, shared by every flow that mails a single-use link:
//! self-service and admin-issued password resets, and address verification.
//!
//! A link token is `<selector>.<verifier>`: the plaintext `selector` indexes the token
//! row and only `SHA-256(verifier)` is persisted, so the token is high-entropy and
//! unguessable — retiring the brute-forceable 6-digit code (CWE-307). Lookups key on
//! `selector` (no global hash scan) and the DB layer bounds wrong-verifier guesses per
//! token.

use pierre_config::constants::password_reset;
use rand::distr::Alphanumeric;
use rand::Rng;
use sha2::{Digest, Sha256};

/// A freshly generated link token: what to persist plus what to deliver.
pub struct GeneratedLinkToken {
    /// Plaintext lookup half — persisted as-is (indexed).
    pub selector: String,
    /// SHA-256 hex of the verifier half — the only secret material persisted.
    pub verifier_hash: String,
    /// The full `<selector>.<verifier>` token to deliver to the user (never stored).
    pub token: String,
}

fn random_alnum(len: usize) -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

fn sha256_hex(input: &str) -> String {
    format!("{:x}", Sha256::digest(input.as_bytes()))
}

/// Generate a new `<selector>.<verifier>` reset token plus the values the DB stores.
#[must_use]
pub fn generate_link_token() -> GeneratedLinkToken {
    let selector = random_alnum(password_reset::SELECTOR_LEN);
    let verifier = random_alnum(password_reset::VERIFIER_LEN);
    let verifier_hash = sha256_hex(&verifier);
    let token = format!(
        "{selector}{delim}{verifier}",
        delim = password_reset::TOKEN_DELIMITER
    );
    GeneratedLinkToken {
        selector,
        verifier_hash,
        token,
    }
}

/// Split a delivered reset token into `(selector, verifier_hash)` for consumption.
///
/// Splits on the first delimiter (selector + verifier are alphanumeric, so there is
/// exactly one). Returns `None` for a malformed token (missing/empty halves); the caller
/// treats that as an invalid token.
#[must_use]
pub fn split_link_token(token: &str) -> Option<(String, String)> {
    let (selector, verifier) = token.split_once(password_reset::TOKEN_DELIMITER)?;
    if selector.is_empty() || verifier.is_empty() {
        return None;
    }
    Some((selector.to_owned(), sha256_hex(verifier)))
}
