// ABOUTME: The Google web client behind "Continue with Google": configured only when whole, never printed or serialized
// ABOUTME: Also pins the redirect URI it registers and the host-only cookie names the hosted login page sets
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The hosted OAuth login page shows "Continue with Google" only when a
//! Google web client is configured (carnet#652). A half-configured client
//! could only fail at Google, after the athlete chose an account, so it is
//! no client at all; and its secret sits in `OAuth2ServerConfig`, which
//! derives `Debug` and `Serialize`, so neither may carry it.

use pierre_auth::config::{GoogleSignInConfig, OAuth2ServerConfig};
use pierre_auth::google_oidc::google_callback_url;
use pierre_auth::security::cookies::host_cookie_name;

const CLIENT_ID: &str = "1234-abc.apps.googleusercontent.com";
const SECRET: &str = "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-secret";

#[test]
fn a_client_is_configured_only_when_both_values_are_set() {
    let config = GoogleSignInConfig::from_values(Some(CLIENT_ID), Some(SECRET)).unwrap();
    assert_eq!(config.client_id, CLIENT_ID);
    assert_eq!(config.client_secret, SECRET);
    assert_eq!(
        config.authorization_endpoint,
        "https://accounts.google.com/o/oauth2/v2/auth"
    );
    assert_eq!(config.token_endpoint, "https://oauth2.googleapis.com/token");
    assert_eq!(
        config.jwks_url,
        "https://www.googleapis.com/oauth2/v3/certs"
    );

    let trimmed =
        GoogleSignInConfig::from_values(Some(&format!("  {CLIENT_ID}\n")), Some(SECRET)).unwrap();
    assert_eq!(trimmed.client_id, CLIENT_ID, "values are trimmed");

    for (client_id, secret) in [
        (None, Some(SECRET)),
        (Some(CLIENT_ID), None),
        (Some("   "), Some(SECRET)),
        (Some(CLIENT_ID), Some("")),
        (None, None),
    ] {
        assert!(
            GoogleSignInConfig::from_values(client_id, secret).is_none(),
            "{client_id:?} / {}",
            secret.map_or("None", |_| "a secret")
        );
    }
    assert!(OAuth2ServerConfig::default().google_sign_in.is_none());
}

#[test]
fn the_secret_is_neither_printed_nor_serialized() {
    let google = GoogleSignInConfig::from_values(Some(CLIENT_ID), Some(SECRET)).unwrap();
    let printed = format!("{google:?}");
    assert!(!printed.contains(SECRET), "{printed}");
    assert!(printed.contains(CLIENT_ID));
    assert!(printed.contains(&format!("secret_length: {}", SECRET.len())));
    assert!(printed.contains(&google.secret_fingerprint()));
    assert_eq!(google.secret_fingerprint().len(), 8);

    let config = OAuth2ServerConfig {
        google_sign_in: Some(Box::new(google)),
        ..OAuth2ServerConfig::default()
    };
    let debug = format!("{config:?}");
    assert!(!debug.contains(SECRET), "{debug}");
    let serialized = serde_json::to_string(&config).unwrap();
    assert!(!serialized.contains(SECRET), "{serialized}");
    assert!(!serialized.contains("google_sign_in"), "{serialized}");
}

#[test]
fn google_returns_to_one_callback_under_the_issuer() {
    assert_eq!(
        google_callback_url("https://app.dravr.ai"),
        "https://app.dravr.ai/oauth2/login/google/callback"
    );
    assert_eq!(
        google_callback_url("https://app.dravr.ai/"),
        "https://app.dravr.ai/oauth2/login/google/callback"
    );
    assert_eq!(
        google_callback_url("http://localhost:8081"),
        "http://localhost:8081/oauth2/login/google/callback"
    );
}

#[test]
fn a_secure_cookie_takes_the_host_prefix() {
    assert_eq!(
        host_cookie_name("pierre_session", true),
        "__Host-pierre_session"
    );
    assert_eq!(host_cookie_name("pierre_session", false), "pierre_session");
}
