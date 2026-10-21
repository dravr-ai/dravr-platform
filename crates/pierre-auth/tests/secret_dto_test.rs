// ABOUTME: Auth request and response DTOs keep their JSON wire format while Debug prints no secret
// ABOUTME: Pins LoginRequest, the first-party token response and the authorization server's token pair
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_auth::dto::auth::{LoginRequest, OAuth2TokenResponse};
use pierre_auth::oauth2_server::{TokenRequest, TokenResponse};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{json, Value};

const SECRET_MARKER: &str = "SecretBox<str>([REDACTED])";

#[test]
fn a_login_request_reads_the_password_and_never_prints_it() {
    let request: LoginRequest = serde_json::from_value(json!({
        "email": "athlete@dravr.ai",
        "password": "correct-horse-battery-1",
    }))
    .unwrap();
    assert_eq!(request.password.expose_secret(), "correct-horse-battery-1");

    let printed = format!("{request:?}");
    assert!(!printed.contains("correct-horse-battery-1"), "{printed}");
    assert!(printed.contains("athlete@dravr.ai"), "{printed}");
    assert_eq!(printed.matches(SECRET_MARKER).count(), 1, "{printed}");
}

#[test]
fn a_token_request_reads_its_client_secret_and_refresh_token() {
    let request: TokenRequest = serde_json::from_value(json!({
        "grant_type": "refresh_token",
        "client_id": "mcp-client",
        "client_secret": "cs-9f00e1",
        "refresh_token": "rt-31aa07",
    }))
    .unwrap();
    assert_eq!(request.client_secret.expose_secret(), "cs-9f00e1");
    assert_eq!(
        request
            .refresh_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("rt-31aa07")
    );
    let printed = format!("{request:?}");
    assert!(!printed.contains("cs-9f00e1"), "{printed}");
    assert!(!printed.contains("rt-31aa07"), "{printed}");
    assert!(printed.contains("mcp-client"), "{printed}");
}

#[test]
fn a_token_response_hands_its_tokens_to_the_client_and_prints_none() {
    let response = OAuth2TokenResponse {
        access_token: SecretString::from("jwt-access-6b20"),
        token_type: "Bearer".to_owned(),
        expires_in: 3600,
        refresh_token: Some(SecretString::from("refresh-0e9c")),
        scope: Some("offline_access".to_owned()),
        user: None,
        csrf_token: None,
    };
    let wire: Value = serde_json::to_value(&response).unwrap();
    assert_eq!(wire["access_token"], "jwt-access-6b20");
    assert_eq!(wire["refresh_token"], "refresh-0e9c");
    // A field the response does not carry stays off the wire, as before.
    assert!(wire.get("csrf_token").is_none(), "{wire}");

    let printed = format!("{response:?}");
    assert!(!printed.contains("jwt-access-6b20"), "{printed}");
    assert!(!printed.contains("refresh-0e9c"), "{printed}");
    assert_eq!(printed.matches(SECRET_MARKER).count(), 2, "{printed}");
}

#[test]
fn the_authorization_server_token_response_keeps_rfc_6749_field_names() {
    let response = TokenResponse {
        access_token: SecretString::from("as-access-71d3"),
        token_type: "Bearer".to_owned(),
        expires_in: 3600,
        scope: None,
        refresh_token: None,
    };
    let wire: Value = serde_json::to_value(&response).unwrap();
    assert_eq!(wire["access_token"], "as-access-71d3");
    assert!(!format!("{response:?}").contains("as-access-71d3"));
}
