// ABOUTME: The A2A client credentials handed out once print a redaction for every secret they carry
// ABOUTME: The JSON a client receives still carries the secret, API key and private key verbatim
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_a2a::client_types::ClientCredentials;
use secrecy::SecretString;
use serde_json::Value;

#[test]
fn client_credentials_print_no_secret_and_serialize_every_one() {
    let credentials = ClientCredentials {
        client_id: "a2a-client-1".to_owned(),
        client_secret: SecretString::from("a2a-secret-e41f"),
        api_key: SecretString::from("a2a-key-7d09"),
        public_key: "ed25519-public".to_owned(),
        private_key: SecretString::from("ed25519-private-33c8"),
        key_type: "ed25519".to_owned(),
    };

    let printed = format!("{credentials:?}");
    for secret in ["a2a-secret-e41f", "a2a-key-7d09", "ed25519-private-33c8"] {
        assert!(!printed.contains(secret), "{secret} leaked: {printed}");
    }
    assert_eq!(
        printed.matches("SecretBox<str>([REDACTED])").count(),
        3,
        "{printed}"
    );
    assert!(printed.contains("ed25519-public"), "{printed}");

    let wire: Value = serde_json::to_value(&credentials).unwrap();
    assert_eq!(wire["client_secret"], "a2a-secret-e41f");
    assert_eq!(wire["api_key"], "a2a-key-7d09");
    assert_eq!(wire["private_key"], "ed25519-private-33c8");
}
