// ABOUTME: Pins the StravaProvider access-token screen: an "at_" prefix or a short token is refused
// ABOUTME: before any request leaves, while a 40-character token reaches the Strava-shaped mock
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Strava access-token validation suite.
//
// The provider refuses a token that carries the `at_` prefix or is shorter than
// 40 characters, and it does so before the request is built, so a refused token
// never reaches Strava. Each case drives the real `StravaProvider` against a
// local mock that counts the requests it receives: a refusal must leave the
// count at zero and carry the exact message, and the 40-character boundary must
// pass through to the mock.
#![cfg(feature = "provider-strava")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Once};

use axum::{routing::get, Json, Router};
use chrono::Utc;
use pierre_config::environment::HttpClientConfig;
use pierre_core::errors::ErrorCode;
use pierre_mcp_server::constants::init_server_config;
use pierre_mcp_server::utils::http_client::initialize_http_clients;
use pierre_providers::core::{FitnessProvider, OAuth2Credentials, ProviderConfig};
use pierre_providers::strava_provider::StravaProvider;
use serde_json::json;
use tokio::net::TcpListener;

static INIT_HTTP_CLIENTS: Once = Once::new();
static INIT_SERVER_CONFIG: Once = Once::new();

const REFUSAL: &str =
    "Invalid Strava access token. Please authenticate with Strava first to access real data.";

fn ensure_http_clients_initialized() {
    INIT_SERVER_CONFIG.call_once(|| {
        let _ = init_server_config();
    });
    INIT_HTTP_CLIENTS.call_once(|| {
        initialize_http_clients(HttpClientConfig::default());
    });
}

/// Start a Strava-shaped `/athlete` mock and return its base URL together with
/// the number of requests it has served.
async fn counting_athlete_mock() -> (String, Arc<AtomicUsize>) {
    // Arc: the counter is shared between the spawned server task that
    // increments it and the test that reads it.
    let hits = Arc::new(AtomicUsize::new(0));
    let route_hits = hits.clone();
    let app = Router::new().route(
        "/athlete",
        get(move || {
            let route_hits = route_hits.clone();
            async move {
                route_hits.fetch_add(1, Ordering::SeqCst);
                Json(json!({ "id": 12345, "firstname": "Jean" }))
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (format!("http://{addr}"), hits)
}

/// A provider pointed at `api_base_url` holding `access_token`, with a
/// far-future expiry so the refresh path never runs.
async fn provider_with_token(api_base_url: String, access_token: &str) -> StravaProvider {
    let provider = StravaProvider::with_config(ProviderConfig {
        name: "strava".to_owned(),
        auth_url: "https://www.strava.com/oauth/authorize".to_owned(),
        token_url: "https://www.strava.com/oauth/token".to_owned(),
        api_base_url,
        revoke_url: None,
        default_scopes: vec!["read".to_owned()],
    });
    provider
        .set_credentials(OAuth2Credentials {
            client_id: "test_client".to_owned(),
            client_secret: "test_secret".to_owned(),
            access_token: Some(access_token.to_owned()),
            refresh_token: Some("test_refresh_token".to_owned()),
            expires_at: Some(Utc::now() + chrono::Duration::days(30)),
            scopes: vec!["read".to_owned()],
        })
        .await
        .expect("set_credentials");
    provider
}

#[tokio::test]
async fn at_prefixed_token_is_refused_before_any_request() {
    ensure_http_clients_initialized();
    let (base_url, hits) = counting_athlete_mock().await;

    let token = format!("at_{}", "a".repeat(40));
    let provider = provider_with_token(base_url, &token).await;
    let error = provider
        .get_athlete()
        .await
        .expect_err("an at_ token is refused whatever its length");

    assert_eq!(error.code, ErrorCode::InternalError);
    assert_eq!(error.message, REFUSAL);
    assert_eq!(hits.load(Ordering::SeqCst), 0, "nothing may reach Strava");
}

#[tokio::test]
async fn token_shorter_than_forty_characters_is_refused_before_any_request() {
    ensure_http_clients_initialized();
    let (base_url, hits) = counting_athlete_mock().await;

    let provider = provider_with_token(base_url, &"a".repeat(39)).await;
    let error = provider
        .get_athlete()
        .await
        .expect_err("a 39-character token is refused");

    assert_eq!(error.code, ErrorCode::InternalError);
    assert_eq!(error.message, REFUSAL);
    assert_eq!(hits.load(Ordering::SeqCst), 0, "nothing may reach Strava");
}

#[tokio::test]
async fn forty_character_token_reaches_strava() {
    ensure_http_clients_initialized();
    let (base_url, hits) = counting_athlete_mock().await;

    let provider = provider_with_token(base_url, &"a".repeat(40)).await;
    let athlete = provider
        .get_athlete()
        .await
        .expect("a 40-character token passes the screen");

    assert_eq!(athlete.id, "12345");
    assert_eq!(athlete.firstname.as_deref(), Some("Jean"));
    assert_eq!(hits.load(Ordering::SeqCst), 1, "exactly one request served");
}
