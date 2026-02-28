// ABOUTME: Tests for Slack webhook HMAC-SHA256 signature verification
// ABOUTME: Validates correct signatures, tampered signatures, missing headers, and replay protection
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use axum::http::HeaderMap;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use pierre_messaging::slack::signature::verify_slack_signature;

/// Build a valid HMAC-SHA256 signature for Slack webhook verification.
///
/// Mirrors the algorithm in `verify_slack_signature`:
///   `base_string` = "v0:{timestamp}:{body}"
///   signature   = "v0=" + hex(HMAC-SHA256(signing_secret, `base_string`))
fn compute_signature(signing_secret: &str, timestamp: &str, body: &str) -> String {
    let base_string = format!("v0:{timestamp}:{body}");
    let mut mac =
        Hmac::<Sha256>::new_from_slice(signing_secret.as_bytes()).expect("valid HMAC key");
    mac.update(base_string.as_bytes());
    let digest = mac.finalize().into_bytes();
    format!("v0={}", hex::encode(digest))
}

/// Return a current Unix timestamp as a string (within the 5-minute window).
fn current_timestamp() -> String {
    chrono::Utc::now().timestamp().to_string()
}

/// Build headers with valid Slack signature and timestamp.
fn signed_headers(signing_secret: &str, timestamp: &str, body: &str) -> HeaderMap {
    let sig = compute_signature(signing_secret, timestamp, body);
    let mut headers = HeaderMap::new();
    headers.insert("x-slack-signature", sig.parse().unwrap());
    headers.insert("x-slack-request-timestamp", timestamp.parse().unwrap());
    headers
}

// ============================================================================
// Valid Signature Tests
// ============================================================================

#[test]
fn test_verify_valid_signature() {
    let secret = "s-test-secret-abc123";
    let body = r#"{"type":"event_callback","event":{"type":"message"}}"#;
    let ts = current_timestamp();

    let headers = signed_headers(secret, &ts, body);
    let result = verify_slack_signature(secret, &headers, body.as_bytes()).unwrap();

    assert!(result, "valid signature should verify successfully");
}

#[test]
fn test_verify_valid_signature_with_different_body() {
    let secret = "another-signing-secret";
    let body = r#"{"token":"xyzzy","challenge":"abc123","type":"url_verification"}"#;
    let ts = current_timestamp();

    let headers = signed_headers(secret, &ts, body);
    let result = verify_slack_signature(secret, &headers, body.as_bytes()).unwrap();

    assert!(result, "signature should verify for any valid body content");
}

// ============================================================================
// Invalid Signature Tests
// ============================================================================

#[test]
fn test_verify_invalid_signature_wrong_secret() {
    let correct_secret = "correct-secret";
    let wrong_secret = "wrong-secret";
    let body = r#"{"type":"event_callback"}"#;
    let ts = current_timestamp();

    // Sign with the wrong secret
    let headers = signed_headers(wrong_secret, &ts, body);
    let result = verify_slack_signature(correct_secret, &headers, body.as_bytes()).unwrap();

    assert!(!result, "signature from wrong secret should not verify");
}

#[test]
fn test_verify_invalid_signature_tampered_body() {
    let secret = "my-secret";
    let original_body = r#"{"type":"event_callback","event":{"text":"hello"}}"#;
    let tampered_body = r#"{"type":"event_callback","event":{"text":"hacked"}}"#;
    let ts = current_timestamp();

    // Sign the original body, but verify against tampered body
    let headers = signed_headers(secret, &ts, original_body);
    let result = verify_slack_signature(secret, &headers, tampered_body.as_bytes()).unwrap();

    assert!(!result, "tampered body should fail verification");
}

#[test]
fn test_verify_invalid_signature_garbage_hex() {
    let secret = "my-secret";
    let body = r#"{"type":"event_callback"}"#;
    let ts = current_timestamp();

    let mut headers = HeaderMap::new();
    headers.insert(
        "x-slack-signature",
        "v0=not_real_hex_garbage".parse().unwrap(),
    );
    headers.insert("x-slack-request-timestamp", ts.parse().unwrap());

    let result = verify_slack_signature(secret, &headers, body.as_bytes()).unwrap();

    assert!(!result, "garbage signature should fail verification");
}

// ============================================================================
// Missing Header Tests
// ============================================================================

#[test]
fn test_verify_missing_signature_header() {
    let secret = "my-secret";
    let body = b"some body";

    let mut headers = HeaderMap::new();
    headers.insert(
        "x-slack-request-timestamp",
        current_timestamp().parse().unwrap(),
    );
    // No x-slack-signature header

    let result = verify_slack_signature(secret, &headers, body);
    assert!(
        result.is_err(),
        "missing signature header should return Err"
    );
}

#[test]
fn test_verify_missing_timestamp_header() {
    let secret = "my-secret";
    let body = b"some body";

    let mut headers = HeaderMap::new();
    headers.insert("x-slack-signature", "v0=abc123".parse().unwrap());
    // No x-slack-request-timestamp header

    let result = verify_slack_signature(secret, &headers, body);
    assert!(
        result.is_err(),
        "missing timestamp header should return Err"
    );
}

#[test]
fn test_verify_empty_headers() {
    let secret = "my-secret";
    let body = b"some body";
    let headers = HeaderMap::new();

    let result = verify_slack_signature(secret, &headers, body);
    assert!(result.is_err(), "empty headers should return Err");
}

#[test]
fn test_verify_invalid_timestamp_format() {
    let secret = "my-secret";
    let body = b"some body";

    let mut headers = HeaderMap::new();
    headers.insert("x-slack-signature", "v0=abc123".parse().unwrap());
    headers.insert("x-slack-request-timestamp", "not-a-number".parse().unwrap());

    let result = verify_slack_signature(secret, &headers, body);
    assert!(result.is_err(), "non-numeric timestamp should return Err");
}

// ============================================================================
// Replay Attack Protection Tests
// ============================================================================

#[test]
fn test_verify_stale_timestamp_rejected() {
    let secret = "my-secret";
    let body = r#"{"type":"event_callback"}"#;
    // 10 minutes ago (600 seconds), well beyond the 300-second limit
    let stale_ts = (chrono::Utc::now().timestamp() - 600).to_string();

    let headers = signed_headers(secret, &stale_ts, body);
    let result = verify_slack_signature(secret, &headers, body.as_bytes());

    assert!(
        result.is_err(),
        "stale timestamp (>5 min) should be rejected"
    );
}

#[test]
fn test_verify_timestamp_at_boundary_accepted() {
    let secret = "my-secret";
    let body = r#"{"type":"event_callback"}"#;
    // Exactly 4 minutes and 50 seconds ago: within the 300-second window
    let ts = (chrono::Utc::now().timestamp() - 290).to_string();

    let headers = signed_headers(secret, &ts, body);
    let result = verify_slack_signature(secret, &headers, body.as_bytes()).unwrap();

    assert!(result, "timestamp within 5 min window should be accepted");
}
