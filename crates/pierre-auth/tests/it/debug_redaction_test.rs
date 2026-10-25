// ABOUTME: Pins every hand-written Debug impl in pierre-auth that hides a credential
// ABOUTME: A future derive(Debug) on one of these types fails here instead of logging a secret
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Each test builds a type with a distinctive sentinel in every secret field,
//! formats it with `{:?}` and `{:#?}`, and asserts no sentinel comes out.

use std::fmt::Debug;

use chrono::Utc;
use pierre_auth::admin::jwks::RsaKeyPair;
use pierre_auth::config::{FirebaseConfig, GoogleSignInConfig};
use pierre_auth::dto::auth::{
    AcceptInvitationRequest, OAuth2TokenRequest, OAuthAuthorizationResponse,
};
use pierre_auth::firebase::FirebaseAuth;
use pierre_auth::google_oidc::{GoogleOidcClient, GoogleSignInStart};
use pierre_auth::human_check::TurnstileCheck;
use pierre_auth::key_management::{
    DatabaseEncryptionKey, KeyManager, LocalKekProvider, MasterEncryptionKey,
};
use pierre_auth::oauth2_client::PkceParams;
use pierre_auth::oauth2_server::{
    AuthorizeRequest, AuthorizeResponse, Authorized, PkceConfig, PkceMethod, TokenRequest,
};
use pierre_auth::security::cookies::{SameSitePolicy, SecureCookieConfig};
use rsa::traits::PrivateKeyParts;
use secrecy::SecretString;

/// Formats `value` both ways, asserts none of `secrets` appears in either,
/// and returns the compact form for further checks.
fn printed_without(value: &impl Debug, secrets: &[&str]) -> String {
    let compact = format!("{value:?}");
    let pretty = format!("{value:#?}");
    for secret in secrets {
        assert!(!compact.contains(secret), "{secret} leaked: {compact}");
        assert!(!pretty.contains(secret), "{secret} leaked: {pretty}");
    }
    compact
}

/// How `{:?}` renders a byte array filled with `byte`, for asserting key
/// material never prints.
fn byte_run(byte: u8) -> String {
    format!("{byte}, {byte}, {byte}, {byte}")
}

#[test]
fn a_session_cookie_never_prints_its_value() {
    let cookie = SecureCookieConfig {
        name: "auth_token".to_owned(),
        value: "sentinel-cookie-7f3a".to_owned(),
        max_age_secs: 3600,
        http_only: true,
        secure: true,
        same_site: SameSitePolicy::Strict,
        path: "/".to_owned(),
    };
    let printed = printed_without(&cookie, &["sentinel-cookie-7f3a"]);
    assert!(printed.contains("value: \"[REDACTED]\""), "{printed}");
    assert!(printed.contains("auth_token"), "{printed}");
}

#[test]
fn an_rsa_key_pair_never_prints_its_private_exponent() {
    let pair = RsaKeyPair::generate_with_key_size("sentinel-kid", 2048).unwrap();
    let private_exponent = pair.private_key.d().to_string();
    let printed = printed_without(&pair, &[&private_exponent]);
    assert!(printed.contains("private_key: \"[REDACTED]\""), "{printed}");
    assert!(printed.contains("sentinel-kid"), "{printed}");
}

#[test]
fn an_invitation_acceptance_never_prints_its_token_or_password() {
    let request = AcceptInvitationRequest {
        token: "sentinel-selector.sentinel-verifier-7f3a".to_owned(),
        password: "sentinel-password-7f3a".to_owned(),
        display_name: Some("Invited Athlete".to_owned()),
    };
    let printed = printed_without(
        &request,
        &[
            "sentinel-selector",
            "sentinel-verifier-7f3a",
            "sentinel-password-7f3a",
        ],
    );
    assert_eq!(printed.matches("[REDACTED]").count(), 2, "{printed}");
    assert!(printed.contains("Invited Athlete"), "{printed}");
}

#[test]
fn a_google_sign_in_start_never_prints_its_state_nonce_verifier_or_url() {
    let start = GoogleSignInStart {
        state: "sentinel-state-7f3a".to_owned(),
        nonce: "sentinel-nonce-7f3a".to_owned(),
        code_verifier: "sentinel-verifier-7f3a".to_owned(),
        authorization_url: "https://accounts.google.com/o/oauth2/v2/auth?state=sentinel-url-7f3a"
            .to_owned(),
    };
    let printed = printed_without(
        &start,
        &[
            "sentinel-state-7f3a",
            "sentinel-nonce-7f3a",
            "sentinel-verifier-7f3a",
            "sentinel-url-7f3a",
        ],
    );
    assert_eq!(printed.matches("[REDACTED]").count(), 3, "{printed}");
}

#[test]
fn a_turnstile_check_never_prints_its_secret_key() {
    let check = TurnstileCheck::new("public-site-key", "sentinel-turnstile-7f3a");
    let printed = printed_without(&check, &["sentinel-turnstile-7f3a"]);
    assert!(printed.contains("secret_key: \"[REDACTED]\""), "{printed}");
    assert!(printed.contains("public-site-key"), "{printed}");
}

#[test]
fn a_master_encryption_key_never_prints_its_bytes() {
    let mek = MasterEncryptionKey::from_bytes([0x7f; 32]);
    let printed = printed_without(&mek, &[&byte_run(0x7f)]);
    assert!(printed.contains("key: \"[REDACTED]\""), "{printed}");
}

#[test]
fn a_database_encryption_key_never_prints_its_bytes() {
    let dek = DatabaseEncryptionKey::from_bytes([0x6e; 32]);
    let printed = printed_without(&dek, &[&byte_run(0x6e)]);
    assert!(printed.contains("key: \"[REDACTED]\""), "{printed}");
}

#[test]
fn a_key_manager_prints_neither_its_master_key_nor_its_data_key() {
    let (manager, dek) = KeyManager::bootstrap_with(Box::new(LocalKekProvider::from_mek(
        MasterEncryptionKey::from_bytes([0x7f; 32]),
    )));
    let dek_bytes = format!("{dek:?}");
    let dek_bytes = dek_bytes.trim_start_matches('[').trim_end_matches(']');
    let printed = printed_without(&manager, &[&byte_run(0x7f), dek_bytes]);
    assert!(printed.contains("[REDACTED]"), "{printed}");
}

#[test]
fn a_google_oidc_client_never_prints_its_client_secret() {
    let client = GoogleOidcClient::new(GoogleSignInConfig::new(
        "client-1.apps.googleusercontent.com",
        "sentinel-google-secret-7f3a",
    ));
    let printed = printed_without(&client, &["sentinel-google-secret-7f3a"]);
    assert!(
        printed.contains("client-1.apps.googleusercontent.com"),
        "{printed}"
    );
}

#[test]
fn firebase_auth_never_prints_its_api_key() {
    let auth = FirebaseAuth::new(FirebaseConfig {
        project_id: Some("dravr-test".to_owned()),
        api_key: Some(SecretString::from("sentinel-firebase-key-7f3a")),
        enabled: true,
        ..FirebaseConfig::default()
    });
    let printed = printed_without(&auth, &["sentinel-firebase-key-7f3a"]);
    assert!(printed.contains("dravr-test"), "{printed}");
}

#[test]
fn a_first_party_token_request_never_prints_its_code_or_verifier() {
    let request = OAuth2TokenRequest {
        grant_type: "authorization_code".to_owned(),
        code: Some("sentinel-code-7f3a".to_owned()),
        redirect_uri: Some("https://app.dravr.ai/callback".to_owned()),
        code_verifier: Some("sentinel-verifier-7f3a".to_owned()),
        refresh_token: Some(SecretString::from("sentinel-refresh-7f3a")),
        client_id: Some("dravr-web".to_owned()),
        scope: None,
        app_attest_key_id: None,
        app_attest_attestation: None,
        app_attest_assertion: None,
        play_integrity_token: Some(SecretString::from("sentinel-integrity-7f3a")),
    };
    let printed = printed_without(
        &request,
        &[
            "sentinel-code-7f3a",
            "sentinel-verifier-7f3a",
            "sentinel-refresh-7f3a",
            "sentinel-integrity-7f3a",
        ],
    );
    assert!(printed.contains("OAuth2TokenRequest"), "{printed}");
    assert!(printed.contains("code: Some(\"[REDACTED]\")"), "{printed}");
    assert!(printed.contains("dravr-web"), "{printed}");
}

#[test]
fn a_provider_authorization_response_never_prints_its_state() {
    let response = OAuthAuthorizationResponse {
        authorization_url: "https://www.strava.com/oauth/authorize?state=sentinel-url-7f3a"
            .to_owned(),
        state: "sentinel-state-7f3a".to_owned(),
        instructions: "Open the link".to_owned(),
        expires_in_minutes: 10,
    };
    let printed = printed_without(&response, &["sentinel-state-7f3a", "sentinel-url-7f3a"]);
    assert!(printed.contains("OAuthAuthorizationResponse"), "{printed}");
    assert!(printed.contains("Open the link"), "{printed}");
}

#[test]
fn pkce_params_never_print_their_verifier() {
    let params = PkceParams {
        code_verifier: "sentinel-verifier-7f3a".to_owned(),
        code_challenge: "public-challenge".to_owned(),
        code_challenge_method: "S256".to_owned(),
    };
    let printed = printed_without(&params, &["sentinel-verifier-7f3a"]);
    assert!(printed.contains("PkceParams"), "{printed}");
    assert!(printed.contains("public-challenge"), "{printed}");
}

#[test]
fn an_authorize_request_never_prints_its_state() {
    let request = AuthorizeRequest {
        response_type: "code".to_owned(),
        client_id: "client-1".to_owned(),
        redirect_uri: "https://app.dravr.ai/callback".to_owned(),
        scope: None,
        state: Some("sentinel-state-7f3a".to_owned()),
        code_challenge: Some("public-challenge".to_owned()),
        code_challenge_method: Some("S256".to_owned()),
        resource: None,
    };
    let printed = printed_without(&request, &["sentinel-state-7f3a"]);
    assert!(printed.contains("AuthorizeRequest"), "{printed}");
    assert!(printed.contains("state: Some(\"[REDACTED]\")"), "{printed}");
}

#[test]
fn an_authorize_response_never_prints_its_code_or_state() {
    let response = AuthorizeResponse {
        code: "sentinel-code-7f3a".to_owned(),
        state: Some("sentinel-state-7f3a".to_owned()),
    };
    let printed = printed_without(&response, &["sentinel-code-7f3a", "sentinel-state-7f3a"]);
    assert!(printed.contains("AuthorizeResponse"), "{printed}");
    assert_eq!(printed.matches("[REDACTED]").count(), 2, "{printed}");
}

#[test]
fn a_token_request_never_prints_its_code_verifier_or_secrets() {
    let request = TokenRequest {
        grant_type: "authorization_code".to_owned(),
        code: Some("sentinel-code-7f3a".to_owned()),
        redirect_uri: None,
        client_id: "client-1".to_owned(),
        client_secret: SecretString::from("sentinel-client-secret-7f3a"),
        scope: None,
        refresh_token: Some(SecretString::from("sentinel-refresh-7f3a")),
        code_verifier: Some("sentinel-verifier-7f3a".to_owned()),
        resource: None,
    };
    let printed = printed_without(
        &request,
        &[
            "sentinel-code-7f3a",
            "sentinel-client-secret-7f3a",
            "sentinel-refresh-7f3a",
            "sentinel-verifier-7f3a",
        ],
    );
    assert!(printed.contains("TokenRequest"), "{printed}");
    assert!(printed.contains("client-1"), "{printed}");
}

#[test]
fn an_authorized_flow_state_never_prints_its_code_state_or_verifier() {
    let authorized = Authorized {
        code: "sentinel-code-7f3a".to_owned(),
        state: Some("sentinel-state-7f3a".to_owned()),
        code_verifier: Some("sentinel-verifier-7f3a".to_owned()),
        expires_at: Utc::now(),
    };
    let printed = printed_without(
        &authorized,
        &[
            "sentinel-code-7f3a",
            "sentinel-state-7f3a",
            "sentinel-verifier-7f3a",
        ],
    );
    assert!(printed.contains("Authorized"), "{printed}");
    assert_eq!(printed.matches("[REDACTED]").count(), 3, "{printed}");
}

#[test]
fn a_pkce_config_never_prints_its_verifier() {
    let config = PkceConfig {
        code_challenge: "public-challenge".to_owned(),
        code_challenge_method: PkceMethod::S256,
        code_verifier: "sentinel-verifier-7f3a".to_owned(),
    };
    let printed = printed_without(&config, &["sentinel-verifier-7f3a"]);
    assert!(printed.contains("PkceConfig"), "{printed}");
    assert!(printed.contains("public-challenge"), "{printed}");
}
