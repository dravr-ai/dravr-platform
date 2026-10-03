// ABOUTME: Pins Garmin's OAuth2 PKCE connect flow: an S256 challenge at oauth2Confirm, the verifier and client in the exchange body
// ABOUTME: Mocks Garmin's token and user-id endpoints; the connection stores the Garmin user id its push events name
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin `OAuth2` PKCE connect suite (carnet#737).
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
//
// Garmin Connect Developer Program "OAuth2.0 PKCE Specification": the user is
// sent to `connect.garmin.com/oauth2Confirm` with `response_type=code`,
// `client_id`, `code_challenge` (S256 of the verifier) and
// `code_challenge_method=S256`; the code is exchanged at
// `diauth.garmin.com/di-oauth2-service/oauth/token` with the client
// credentials, the code and the `code_verifier` in the form body. The token
// response names no user, so the user id is read at `GET user/id`. The
// exchange and the user-id read reach a local mock through the seams
// production leaves unset (`PIERRE_GARMIN_TOKEN_URL`,
// `PIERRE_GARMIN_API_BASE_URL`).
#![cfg(feature = "provider-garmin")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::collections::HashMap;
use std::env;
use std::sync::{Arc, Mutex};

use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_services::oauth_flow::{AuthUrlOptions, OAuthService};
use serde_json::json;
use serial_test::serial;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use url::{form_urlencoded, Url};

/// The Garmin user id the mocked `user/id` reports.
const GARMIN_USER_ID: &str = "garmin-user-0737";
const CLIENT_ID: &str = "garmin_client";
const CLIENT_SECRET: &str = "garmin_secret";

/// Environment set for the duration of one test and removed after it.
struct EnvGuard {
    keys: Vec<&'static str>,
}

impl EnvGuard {
    fn set(vars: &[(&'static str, String)]) -> Self {
        for (key, value) in vars {
            env::set_var(key, value);
        }
        Self {
            keys: vars.iter().map(|(key, _)| *key).collect(),
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for key in &self.keys {
            env::remove_var(key);
        }
    }
}

/// What the Garmin-shaped mock saw.
#[derive(Default)]
struct MockGarmin {
    /// The form body of every token request.
    token_forms: Mutex<Vec<HashMap<String, String>>>,
    /// The `Authorization` header of every `user/id` read.
    user_id_bearers: Mutex<Vec<String>>,
}

/// Stand up a mock Garmin: the token endpoint answers as Garmin documents it
/// (no user id), and `GET /user/id` answers the user id.
async fn mock_garmin() -> (String, Arc<MockGarmin>) {
    let recorder = Arc::new(MockGarmin::default());
    let token_recorder = Arc::clone(&recorder);
    let user_recorder = Arc::clone(&recorder);
    let app = Router::new()
        .route(
            "/di-oauth2-service/oauth/token",
            post(move |body: String| {
                let recorder = Arc::clone(&token_recorder);
                async move {
                    let form = form_urlencoded::parse(body.as_bytes())
                        .into_owned()
                        .collect();
                    recorder.token_forms.lock().unwrap().push(form);
                    Json(json!({
                        "access_token": "garmin_new_access",
                        "token_type": "bearer",
                        "refresh_token": "garmin_new_refresh",
                        "expires_in": 86_400,
                        "scope": "PARTNER_WRITE PARTNER_READ CONNECT_READ CONNECT_WRITE",
                        "jti": "garmin-jti",
                        "refresh_token_expires_in": 7_775_999
                    }))
                }
            }),
        )
        .route(
            "/user/id",
            get(move |headers: HeaderMap| {
                let recorder = Arc::clone(&user_recorder);
                async move {
                    let bearer = headers
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_owned();
                    recorder.user_id_bearers.lock().unwrap().push(bearer);
                    Json(json!({ "userId": GARMIN_USER_ID }))
                }
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (format!("http://{addr}"), recorder)
}

/// A server context whose Garmin token exchange and API reads reach the mock.
/// The registry reads the overrides when the context is built, so the guard
/// is created first.
async fn context_pointed_at(base: &str) -> (Arc<ServerContext>, EnvGuard) {
    let guard = EnvGuard::set(&[
        (
            "PIERRE_GARMIN_TOKEN_URL",
            format!("{base}/di-oauth2-service/oauth/token"),
        ),
        ("PIERRE_GARMIN_API_BASE_URL", base.to_owned()),
        ("GARMIN_CLIENT_ID", CLIENT_ID.to_owned()),
        ("GARMIN_CLIENT_SECRET", CLIENT_SECRET.to_owned()),
    ]);
    let resources = common::create_test_server_resources().await.unwrap();
    (resources, guard)
}

/// The value of query parameter `key` in `url`.
fn query(url: &Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
}

/// Connecting Garmin sends the athlete to Garmin's `OAuth2` PKCE authorization
/// with an S256 challenge, and the callback exchanges the code with the
/// verifier that challenge was made from, the client credentials in the form
/// body, and the redirect URI of the authorization. The token is stored with
/// the Garmin user id read with the fresh access token.
#[tokio::test]
#[serial]
async fn garmin_connect_is_oauth2_pkce_and_records_the_garmin_user_id() {
    let (base, mock) = mock_garmin().await;
    let (resources, _env) = context_pointed_at(&base).await;
    let (user_id, _user, tenant_id) = common::create_test_user_with_plan(
        &resources.agent.database,
        "garmin-pkce@example.com",
        "starter",
    )
    .await
    .unwrap();
    let service = OAuthService::new(resources.data(), resources.common.config.clone());

    // The authorization: Garmin's oauth2Confirm, with an S256 challenge.
    let authorization = service
        .get_auth_url(user_id, tenant_id, "garmin", AuthUrlOptions::default())
        .await
        .expect("a Garmin authorization URL");
    let url = Url::parse(&authorization.authorization_url).unwrap();
    assert_eq!(
        format!("{}{}", url.origin().ascii_serialization(), url.path()),
        "https://connect.garmin.com/oauth2Confirm"
    );
    assert_eq!(query(&url, "response_type").as_deref(), Some("code"));
    assert_eq!(query(&url, "client_id").as_deref(), Some(CLIENT_ID));
    assert_eq!(
        query(&url, "code_challenge_method").as_deref(),
        Some("S256")
    );
    // Garmin's scope is fixed server-side: the authorization carries no
    // `scope` parameter at all, not an empty one.
    assert_eq!(query(&url, "scope"), None);
    assert!(!authorization.authorization_url.contains("scope="));
    let challenge = query(&url, "code_challenge").expect("a PKCE code_challenge");
    let redirect_uri = query(&url, "redirect_uri").expect("a redirect_uri");

    // The callback: the code is exchanged at the token endpoint.
    service
        .handle_callback("garmin-auth-code", &authorization.state, "garmin")
        .await
        .expect("the exchange succeeds against the mock token endpoint");

    let forms = mock.token_forms.lock().unwrap().clone();
    assert_eq!(forms.len(), 1, "one code exchange");
    let form = &forms[0];
    assert_eq!(
        form.get("grant_type").map(String::as_str),
        Some("authorization_code")
    );
    assert_eq!(
        form.get("code").map(String::as_str),
        Some("garmin-auth-code")
    );
    assert_eq!(form.get("client_id").map(String::as_str), Some(CLIENT_ID));
    assert_eq!(
        form.get("client_secret").map(String::as_str),
        Some(CLIENT_SECRET),
        "Garmin takes the client credentials in the form body"
    );
    assert_eq!(form.get("redirect_uri"), Some(&redirect_uri));
    let verifier = form
        .get("code_verifier")
        .expect("the exchange carries the PKCE code_verifier");
    assert_eq!(
        URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        challenge,
        "the verifier is the one the authorization's S256 challenge was made from"
    );

    // The token names no user: the id is read with the fresh access token.
    assert_eq!(
        *mock.user_id_bearers.lock().unwrap(),
        vec!["Bearer garmin_new_access".to_owned()]
    );
    let stored = resources
        .common
        .repos
        .oauth_tokens
        .get_token(user_id, tenant_id, "garmin")
        .await
        .unwrap()
        .expect("a garmin token row exists");
    assert_eq!(stored.access_token, "garmin_new_access");
    assert_eq!(stored.refresh_token.as_deref(), Some("garmin_new_refresh"));
    assert_eq!(
        stored.provider_user_id.as_deref(),
        Some(GARMIN_USER_ID),
        "the Garmin user id is stored with the token"
    );
}

/// Omitting an empty scope leaves the providers that request scopes alone:
/// Strava's and WHOOP's authorizations still carry theirs.
#[tokio::test]
#[serial]
async fn strava_and_whoop_authorizations_still_carry_their_scope() {
    let _env = EnvGuard::set(&[
        ("STRAVA_CLIENT_ID", "strava_client".to_owned()),
        ("STRAVA_CLIENT_SECRET", "strava_secret".to_owned()),
        ("WHOOP_CLIENT_ID", "whoop_client".to_owned()),
        ("WHOOP_CLIENT_SECRET", "whoop_secret".to_owned()),
    ]);
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, _user, tenant_id) = common::create_test_user_with_plan(
        &resources.agent.database,
        "scoped-providers@example.com",
        "starter",
    )
    .await
    .unwrap();
    let service = OAuthService::new(resources.data(), resources.common.config.clone());

    for (provider, expected) in [
        ("strava", "activity:read_all"),
        (
            "whoop",
            "offline read:profile read:body_measurement read:workout read:sleep read:recovery read:cycles",
        ),
    ] {
        let authorization = service
            .get_auth_url(user_id, tenant_id, provider, AuthUrlOptions::default())
            .await
            .unwrap_or_else(|e| panic!("a {provider} authorization URL: {e}"));
        let url = Url::parse(&authorization.authorization_url).unwrap();
        assert_eq!(
            query(&url, "scope").as_deref(),
            Some(expected),
            "{provider} still requests its scopes"
        );
    }
}
