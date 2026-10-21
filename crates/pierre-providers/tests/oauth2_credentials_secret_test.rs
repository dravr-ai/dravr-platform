// ABOUTME: Provider OAuth2 credentials print a redaction for the client secret and both tokens
// ABOUTME: Their serialized form still carries every value, so stored and cached credentials read back whole
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_providers::core::{CredentialKind, OAuth2Credentials};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;

fn credentials() -> OAuth2Credentials {
    OAuth2Credentials {
        client_id: "strava-client-1".to_owned(),
        client_secret: SecretString::from("strava-client-secret-a01"),
        access_token: Some(SecretString::from("strava-access-b02")),
        refresh_token: Some(SecretString::from("strava-refresh-c03")),
        expires_at: None,
        scopes: vec!["activity:read_all".to_owned()],
        kind: CredentialKind::default(),
        request_budget: None,
    }
}

#[test]
fn debug_prints_no_client_secret_and_no_token() {
    let printed = format!("{:?}", credentials());
    for secret in [
        "strava-client-secret-a01",
        "strava-access-b02",
        "strava-refresh-c03",
    ] {
        assert!(!printed.contains(secret), "{secret} leaked: {printed}");
    }
    assert_eq!(
        printed.matches("SecretBox<str>([REDACTED])").count(),
        3,
        "{printed}"
    );
    assert!(printed.contains("strava-client-1"), "{printed}");
}

#[test]
fn the_serialized_credentials_read_back_whole() {
    let json: Value = serde_json::to_value(credentials()).unwrap();
    assert_eq!(json["client_secret"], "strava-client-secret-a01");
    assert_eq!(json["access_token"], "strava-access-b02");
    let read: OAuth2Credentials = serde_json::from_value(json).unwrap();
    assert_eq!(
        read.client_secret.expose_secret(),
        "strava-client-secret-a01"
    );
    assert_eq!(
        read.refresh_token.as_ref().map(ExposeSecret::expose_secret),
        Some("strava-refresh-c03")
    );
}
