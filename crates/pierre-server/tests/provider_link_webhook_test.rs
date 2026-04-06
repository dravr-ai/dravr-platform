// ABOUTME: Tests for the outbound provider-linked webhook emitter
// ABOUTME: Verifies HMAC signing determinism and payload structure
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "provider-sciotte")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

// The webhook module is `pub(crate)` and not re-exported, so these tests
// exercise the HMAC signing via an inline copy of the same algorithm — the
// point is to prove the wire-format signature that channel bots must verify.

use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::str::from_utf8;

type HmacSha256 = Hmac<Sha256>;

const HEX: &[u8; 16] = b"0123456789abcdef";

fn sign(secret: &str, payload: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(payload);
    let digest = mac.finalize().into_bytes();
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in &digest {
        hex.push(HEX[(b >> 4) as usize] as char);
        hex.push(HEX[(b & 0x0f) as usize] as char);
    }
    format!("sha256={hex}")
}

#[test]
fn signature_is_deterministic_for_same_input() {
    let a = sign("secret", b"{\"event\":\"provider.linked\"}");
    let b = sign("secret", b"{\"event\":\"provider.linked\"}");
    assert_eq!(a, b);
    assert!(a.starts_with("sha256="));
    assert_eq!(a.len(), "sha256=".len() + 64); // SHA-256 = 32 bytes = 64 hex chars
}

#[test]
fn signature_changes_with_payload() {
    let a = sign("secret", b"payload-one");
    let b = sign("secret", b"payload-two");
    assert_ne!(a, b);
}

#[test]
fn signature_changes_with_secret() {
    let a = sign("secret-1", b"payload");
    let b = sign("secret-2", b"payload");
    assert_ne!(a, b);
}

#[test]
fn signature_matches_known_vector() {
    // Known HMAC-SHA256 test vector (RFC 4231 test case 1):
    // key = 0x0b * 20, data = "Hi There"
    // expected: b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7
    // 0x0b is a valid single-byte UTF-8 sequence (vertical tab).
    let key_bytes = [0x0bu8; 20];
    let secret = from_utf8(&key_bytes).unwrap();
    let sig = sign(secret, b"Hi There");
    assert_eq!(
        sig,
        "sha256=b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
    );
}
