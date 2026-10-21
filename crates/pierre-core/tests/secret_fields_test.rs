// ABOUTME: The token and credential models print a redaction under Debug and keep their wire format
// ABOUTME: Pins UserOAuthToken, TenantOAuthCredentials and the serialize_exposed helpers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::models::{TenantId, TenantOAuthCredentials, UserOAuthToken};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use uuid::Uuid;

const ACCESS: &str = "strava-access-7c1e90";
const REFRESH: &str = "strava-refresh-04b2d8";
const SECRET_MARKER: &str = "SecretBox<str>([REDACTED])";

fn token() -> UserOAuthToken {
    UserOAuthToken::new(
        Uuid::new_v4(),
        "tenant-a".to_owned(),
        "strava".to_owned(),
        ACCESS.to_owned(),
        Some(REFRESH.to_owned()),
        None,
        Some("read".to_owned()),
    )
}

#[test]
fn a_stored_token_never_prints_its_access_or_refresh_token() {
    let printed = format!("{:?}", token());
    assert!(!printed.contains(ACCESS), "{printed}");
    assert!(!printed.contains(REFRESH), "{printed}");
    assert_eq!(printed.matches(SECRET_MARKER).count(), 2, "{printed}");
    assert!(printed.contains("strava"), "{printed}");
}

#[test]
fn a_stored_token_keeps_its_wire_format() {
    let json: Value = serde_json::to_value(token()).unwrap();
    assert_eq!(json["access_token"], ACCESS);
    assert_eq!(json["refresh_token"], REFRESH);

    let read: UserOAuthToken = serde_json::from_value(json).unwrap();
    assert_eq!(read.access_token.expose_secret(), ACCESS);
    assert_eq!(
        read.refresh_token.as_ref().map(ExposeSecret::expose_secret),
        Some(REFRESH)
    );
}

#[test]
fn a_missing_refresh_token_serializes_as_null() {
    let mut stored = token();
    stored.refresh_token = None;
    let json: Value = serde_json::to_value(&stored).unwrap();
    assert_eq!(json["refresh_token"], Value::Null);
}

#[test]
fn tenant_oauth_credentials_never_print_the_client_secret() {
    let credentials = TenantOAuthCredentials {
        tenant_id: TenantId::from_uuid(Uuid::new_v4()),
        provider: "whoop".to_owned(),
        client_id: "whoop-client".to_owned(),
        client_secret: SecretString::from("whoop-secret-55aa01"),
        redirect_uri: "https://app.dravr.ai/api/oauth/callback/whoop".to_owned(),
        scopes: vec!["read:recovery".to_owned()],
        rate_limit_per_day: 1000,
    };
    let printed = format!("{credentials:#?}");
    assert!(!printed.contains("whoop-secret-55aa01"), "{printed}");
    assert_eq!(printed.matches(SECRET_MARKER).count(), 1, "{printed}");
    assert!(printed.contains("whoop-client"), "{printed}");
}
