// ABOUTME: The A2A client credentials handed out once print a redaction for every secret they carry
// ABOUTME: The JSON a client receives carries its id and secret verbatim, and no other credential
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
    };

    let printed = format!("{credentials:?}");
    assert!(
        !printed.contains("a2a-secret-e41f"),
        "the secret leaked: {printed}"
    );
    assert_eq!(
        printed.matches("SecretBox<str>([REDACTED])").count(),
        1,
        "{printed}"
    );

    let wire: Value = serde_json::to_value(&credentials).unwrap();
    assert_eq!(
        wire,
        serde_json::json!({
            "client_id": "a2a-client-1",
            "client_secret": "a2a-secret-e41f",
        }),
        "a client is handed its id and its secret, and no other credential"
    );
}
