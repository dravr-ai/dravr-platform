// ABOUTME: Slack webhook request signature verification using HMAC-SHA256
// ABOUTME: Validates X-Slack-Signature header against request body to prevent forgery
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::http::HeaderMap;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::errors::AppError;

/// Header name for the Slack request signature
const SLACK_SIGNATURE_HEADER: &str = "x-slack-signature";

/// Header name for the Slack request timestamp
const SLACK_TIMESTAMP_HEADER: &str = "x-slack-request-timestamp";

/// Signing protocol version used by Slack
const SLACK_SIGNING_VERSION: &str = "v0";

/// Maximum age of a request timestamp before it's rejected (5 minutes in seconds)
const MAX_TIMESTAMP_AGE_SECS: u64 = 300;

/// Verify a Slack webhook request's HMAC-SHA256 signature
///
/// Slack signs every webhook request using the app's signing secret. This function
/// reconstructs the expected signature and compares it to the one provided in the
/// `X-Slack-Signature` header.
///
/// See: <https://api.slack.com/authentication/verifying-requests-from-slack>
///
/// # Errors
///
/// Returns an error if required headers are missing, the timestamp is too old
/// (replay attack protection), or the signature doesn't match.
pub fn verify_slack_signature(
    signing_secret: &str,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<bool, AppError> {
    // Extract required headers
    let signature = headers
        .get(SLACK_SIGNATURE_HEADER)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::invalid_input("Missing X-Slack-Signature header"))?;

    let timestamp_str = headers
        .get(SLACK_TIMESTAMP_HEADER)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::invalid_input("Missing X-Slack-Request-Timestamp header"))?;

    let timestamp: u64 = timestamp_str.parse().map_err(|_| {
        AppError::invalid_input("Invalid X-Slack-Request-Timestamp: not a valid integer")
    })?;

    // Replay attack protection: reject requests older than 5 minutes
    let now = chrono::Utc::now().timestamp();
    // Safe cast: Unix timestamps are positive and fit in u64 for the foreseeable future
    #[allow(clippy::cast_sign_loss)]
    let now_unsigned = now as u64;

    let age = now_unsigned.saturating_sub(timestamp);
    if age > MAX_TIMESTAMP_AGE_SECS {
        return Err(AppError::invalid_input(
            "Request timestamp is too old (possible replay attack)",
        ));
    }

    // Construct the base string: "v0:timestamp:body"
    let body_str = std::str::from_utf8(body)
        .map_err(|_| AppError::invalid_input("Request body is not valid UTF-8"))?;
    let base_string = format!("{SLACK_SIGNING_VERSION}:{timestamp_str}:{body_str}");

    // Compute HMAC-SHA256
    let mut mac = Hmac::<Sha256>::new_from_slice(signing_secret.as_bytes())
        .map_err(|_| AppError::internal("Failed to create HMAC key"))?;
    mac.update(base_string.as_bytes());
    let computed = mac.finalize().into_bytes();

    // Format as "v0=hex_digest" for comparison
    let computed_signature = format!("{SLACK_SIGNING_VERSION}={}", hex::encode(computed));

    // Constant-time comparison to prevent timing attacks
    Ok(subtle::ConstantTimeEq::ct_eq(
        computed_signature.as_bytes(),
        signature.as_bytes(),
    )
    .into())
}
