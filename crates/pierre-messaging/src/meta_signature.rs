// ABOUTME: Shared Meta Platform HMAC-SHA256 signature verification for Messenger and WhatsApp
// ABOUTME: Both platforms use identical x-hub-signature-256 header with sha256={hex} format
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use hmac::{Hmac, Mac};
use http::HeaderMap;
use pierre_core::errors::messaging::{MessagingError, MessagingResult};
use sha2::Sha256;

/// Verify the `x-hub-signature-256` HMAC-SHA256 header used by Meta platforms
///
/// Both Messenger and `WhatsApp` Cloud API sign webhook payloads with the same
/// scheme: `HMAC-SHA256(app_secret, raw_body)` sent as `sha256={hex}` in the
/// `x-hub-signature-256` header. This function centralises that verification.
///
/// # Errors
///
/// Returns `MessagingError::SignatureVerificationFailed` if the header is missing,
/// malformed (no `sha256=` prefix), or the HMAC does not match.
pub fn verify_meta_signature(
    channel: &str,
    app_secret: &str,
    headers: &HeaderMap,
    body: &[u8],
) -> MessagingResult<()> {
    let signature_header = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| MessagingError::SignatureVerificationFailed {
            channel: channel.to_owned(),
            reason: "missing x-hub-signature-256 header".to_owned(),
        })?;

    // Parse "sha256={hex}" format
    let hex_sig = signature_header.strip_prefix("sha256=").ok_or_else(|| {
        MessagingError::SignatureVerificationFailed {
            channel: channel.to_owned(),
            reason: "signature header must start with sha256=".to_owned(),
        }
    })?;

    // Compute HMAC-SHA256
    let mut mac = Hmac::<Sha256>::new_from_slice(app_secret.as_bytes()).map_err(|e| {
        MessagingError::SignatureVerificationFailed {
            channel: channel.to_owned(),
            reason: format!("HMAC key error: {e}"),
        }
    })?;
    mac.update(body);
    let expected = hex::encode(mac.finalize().into_bytes());

    // Constant-time comparison
    let equal: bool = subtle::ConstantTimeEq::ct_eq(hex_sig.as_bytes(), expected.as_bytes()).into();
    if equal {
        Ok(())
    } else {
        Err(MessagingError::SignatureVerificationFailed {
            channel: channel.to_owned(),
            reason: "signature mismatch".to_owned(),
        })
    }
}
