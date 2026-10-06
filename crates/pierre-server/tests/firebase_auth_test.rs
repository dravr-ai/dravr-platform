// ABOUTME: Tests for Firebase authentication — config, and ID-token validation against a JWK-served key set
// ABOUTME: A test signer plays Google's securetoken key set; each Firebase claim check and key-set failure is pinned
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod helpers;

use std::collections::HashMap;

use dravr_tronc::iam::GoogleKeySet;
use pierre_auth::firebase::{FirebaseAuth, FIREBASE_JWKS_URL};
use pierre_config::environment::FirebaseConfig;
use pierre_core::errors::ErrorCode;
use serde::Serialize;
use serde_json::{json, Value};

use crate::helpers::google_token::{now_secs, unreachable_key_set_url, TestSigner};

const PROJECT: &str = "dravr-test-project";

#[test]
fn test_firebase_config_is_configured() {
    let config = FirebaseConfig {
        project_id: Some("test-project".to_owned()),
        api_key: None,
        enabled: true,
        ..FirebaseConfig::default()
    };
    assert!(config.is_configured());

    let disabled = FirebaseConfig {
        project_id: Some("test-project".to_owned()),
        api_key: None,
        enabled: false,
        ..FirebaseConfig::default()
    };
    assert!(!disabled.is_configured());

    let no_project = FirebaseConfig {
        project_id: None,
        api_key: None,
        enabled: true,
        ..FirebaseConfig::default()
    };
    assert!(!no_project.is_configured());
}

#[test]
fn test_firebase_config_defaults() {
    let config = FirebaseConfig::default();
    assert!(!config.is_configured());
    assert!(config.project_id.is_none());
    assert!(!config.enabled);
}

/// Pinned on purpose: the endpoint is Google's, published in the Firebase
/// documentation for verifying ID tokens. A different URL verifies against
/// keys that never signed a Firebase token.
#[test]
fn production_reads_googles_firebase_jwk_set() {
    assert_eq!(
        FIREBASE_JWKS_URL,
        "https://www.googleapis.com/service_accounts/v1/jwk/securetoken@system.gserviceaccount.com"
    );
}

/// The claims of a Firebase ID token, as Firebase mints them.
#[derive(Debug, Clone, Serialize)]
struct FirebaseTokenClaims {
    iss: String,
    aud: String,
    sub: String,
    iat: u64,
    exp: u64,
    email: Option<String>,
    email_verified: Option<bool>,
    firebase: HashMap<String, Value>,
}

impl FirebaseTokenClaims {
    /// A Google sign-in to `PROJECT`, valid for an hour.
    fn google_sign_in() -> Self {
        let now = now_secs();
        Self {
            iss: format!("https://securetoken.google.com/{PROJECT}"),
            aud: PROJECT.to_owned(),
            sub: "firebase-uid-0001".to_owned(),
            iat: now,
            exp: now + 3600,
            email: Some("athlete@example.com".to_owned()),
            email_verified: Some(true),
            firebase: HashMap::from([("sign_in_provider".to_owned(), json!("google.com"))]),
        }
    }
}

fn firebase_reading(jwks_url: &str) -> FirebaseAuth {
    FirebaseAuth::with_key_set(
        FirebaseConfig {
            project_id: Some(PROJECT.to_owned()),
            api_key: None,
            enabled: true,
            ..FirebaseConfig::default()
        },
        GoogleKeySet::new(jwks_url, reqwest::Client::new()),
    )
}

#[tokio::test]
async fn a_jwk_served_firebase_token_validates_to_its_claims() {
    let signer = TestSigner::generate();
    let jwks = signer.serve_jwks().await;
    let firebase = firebase_reading(&jwks.url);

    let claims = firebase
        .validate_token(&signer.mint(&FirebaseTokenClaims::google_sign_in()))
        .await
        .expect("a token for this project signed by a published key validates");
    assert_eq!(claims.sub, "firebase-uid-0001");
    assert_eq!(claims.aud, PROJECT);
    assert_eq!(claims.email.as_deref(), Some("athlete@example.com"));
    assert_eq!(claims.provider, "google.com");
    assert_eq!(jwks.fetches(), 1);
}

#[tokio::test]
async fn a_firebase_token_for_another_project_or_issuer_is_a_401() {
    let signer = TestSigner::generate();
    let jwks = signer.serve_jwks().await;
    let firebase = firebase_reading(&jwks.url);

    let mut other_project = FirebaseTokenClaims::google_sign_in();
    other_project.aud = "someone-elses-project".to_owned();
    let err = firebase
        .validate_token(&signer.mint(&other_project))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::AuthInvalid);
    assert_eq!(err.message, "Invalid token audience");

    let mut other_issuer = FirebaseTokenClaims::google_sign_in();
    other_issuer.iss = "https://securetoken.google.com/someone-elses-project".to_owned();
    let err = firebase
        .validate_token(&signer.mint(&other_issuer))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::AuthInvalid);
    assert_eq!(err.message, "Invalid token issuer");

    // Signed by a key the set does not publish, under a kid it does.
    let impostor = TestSigner::generate();
    let err = firebase
        .validate_token(
            &impostor.mint_with_kid(&FirebaseTokenClaims::google_sign_in(), &signer.kid),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::AuthInvalid);
    assert_eq!(err.message, "Invalid token");
}

#[tokio::test]
async fn an_expired_firebase_token_is_auth_expired_not_invalid() {
    let signer = TestSigner::generate();
    let jwks = signer.serve_jwks().await;
    let firebase = firebase_reading(&jwks.url);

    let mut expired = FirebaseTokenClaims::google_sign_in();
    expired.exp = now_secs() - 120;
    expired.iat = now_secs() - 3720;
    let err = firebase
        .validate_token(&signer.mint(&expired))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::AuthExpired, "{err:?}");
}

#[tokio::test]
async fn unknown_firebase_key_ids_inside_the_refetch_interval_cost_one_fetch() {
    let signer = TestSigner::generate();
    let jwks = signer.serve_jwks().await;
    let firebase = firebase_reading(&jwks.url);
    let claims = FirebaseTokenClaims::google_sign_in();

    for attempt in 0..5 {
        let err = firebase
            .validate_token(&signer.mint_with_kid(&claims, &format!("made-up-{attempt}")))
            .await
            .expect_err("an unknown kid is refused");
        assert_eq!(err.code, ErrorCode::AuthInvalid, "attempt {attempt}");
        assert_eq!(err.message, "Invalid token");
    }
    assert_eq!(jwks.fetches(), 1, "five unknown kids, one fetch");

    let validated = firebase.validate_token(&signer.mint(&claims)).await;
    assert!(validated.is_ok(), "{validated:?}");
    assert_eq!(jwks.fetches(), 1, "the published kid came from that fetch");
}

#[tokio::test]
async fn an_unreachable_firebase_key_set_is_an_internal_error() {
    let firebase = firebase_reading(&unreachable_key_set_url());
    let signer = TestSigner::generate();

    let err = firebase
        .validate_token(&signer.mint(&FirebaseTokenClaims::google_sign_in()))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InternalError, "{err:?}");
}

#[tokio::test]
async fn a_disabled_or_unconfigured_firebase_refuses_before_any_fetch() {
    let signer = TestSigner::generate();
    let jwks = signer.serve_jwks().await;
    let unconfigured = FirebaseAuth::with_key_set(
        FirebaseConfig::default(),
        GoogleKeySet::new(&jwks.url, reqwest::Client::new()),
    );

    let err = unconfigured
        .validate_token(&signer.mint(&FirebaseTokenClaims::google_sign_in()))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert_eq!(jwks.fetches(), 0);
}
