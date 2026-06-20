// ABOUTME: Regression test for get_stats returning Strava year-to-date totals distinctly from all-time
// ABOUTME: Stands up a Strava-shaped mock server and drives the real StravaProvider end-to-end
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Strava `get_stats` year-to-date regression suite.
//
// Guards the bug where a user asking for their annual distance received their
// all-time distance: the provider previously deserialized only `all_*_totals`
// and dropped Strava's `ytd_*_totals`. This points the real provider at a local
// mock that returns distinct lifetime and year-to-date totals and asserts both
// survive into the canonical `Stats`.
#![cfg(feature = "provider-strava")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use axum::{routing::get, Json, Router};
use chrono::Utc;
use pierre_config::environment::HttpClientConfig;
use pierre_mcp_server::constants::init_server_config;
use pierre_mcp_server::utils::http_client::initialize_http_clients;
use pierre_providers::core::{FitnessProvider, OAuth2Credentials, ProviderConfig};
use pierre_providers::strava_provider::StravaProvider;
use serde_json::{json, Value};
use std::sync::Once;
use tokio::net::TcpListener;

static INIT_HTTP_CLIENTS: Once = Once::new();
static INIT_SERVER_CONFIG: Once = Once::new();

fn ensure_http_clients_initialized() {
    INIT_SERVER_CONFIG.call_once(|| {
        let _ = init_server_config();
    });
    INIT_HTTP_CLIENTS.call_once(|| {
        initialize_http_clients(HttpClientConfig::default());
    });
}

/// Strava athlete-stats payload with deliberately different all-time and
/// year-to-date totals so a swap between the two is detectable.
fn stats_payload() -> Value {
    json!({
        "all_ride_totals": { "count": 100, "distance": 5_000_000.0, "moving_time": 360_000, "elevation_gain": 50_000.0 },
        "all_run_totals":  { "count": 50,  "distance": 2_000_000.0, "moving_time": 180_000, "elevation_gain": 20_000.0 },
        "ytd_ride_totals": { "count": 20,  "distance": 1_000_000.0, "moving_time": 72_000,  "elevation_gain": 10_000.0 },
        "ytd_run_totals":  { "count": 10,  "distance": 400_000.0,   "moving_time": 36_000,  "elevation_gain": 4_000.0 },
    })
}

#[tokio::test]
async fn get_stats_returns_year_to_date_distinct_from_all_time() {
    ensure_http_clients_initialized();

    // Mock the two Strava endpoints get_stats touches: athlete lookup (for the
    // id) and the stats aggregate. The id is fixed so no path param is needed.
    let app = Router::new()
        .route("/athlete", get(|| async { Json(json!({ "id": 12345 })) }))
        .route(
            "/athletes/12345/stats",
            get(|| async { Json(stats_payload()) }),
        );

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let config = ProviderConfig {
        name: "strava".to_owned(),
        auth_url: "https://www.strava.com/oauth/authorize".to_owned(),
        token_url: "https://www.strava.com/oauth/token".to_owned(),
        api_base_url: format!("http://{addr}"),
        revoke_url: None,
        default_scopes: vec!["read".to_owned()],
    };
    let provider = StravaProvider::with_config(config);

    // A token >= 40 chars and not prefixed "at_" passes validate_access_token;
    // a far-future expiry skips the refresh path so no token endpoint is needed.
    provider
        .set_credentials(OAuth2Credentials {
            client_id: "test_client".to_owned(),
            client_secret: "test_secret".to_owned(),
            access_token: Some("test_access_token_0123456789abcdef0123456789".to_owned()),
            refresh_token: Some("test_refresh_token".to_owned()),
            expires_at: Some(Utc::now() + chrono::Duration::days(30)),
            scopes: vec!["read".to_owned()],
        })
        .await
        .expect("set_credentials");

    let stats = provider.get_stats().await.expect("get_stats succeeds");

    // All-time is ride+run lifetime totals.
    assert_eq!(stats.total_activities, 150, "all-time activity count");
    assert!(
        (stats.total_distance - 7_000_000.0).abs() < f64::EPSILON,
        "all-time distance is ride+run lifetime, got {}",
        stats.total_distance
    );

    // Year-to-date is present and is the smaller ride+run current-year subset.
    let ytd = stats
        .year_to_date
        .expect("year_to_date populated for Strava");
    assert_eq!(ytd.total_activities, 30, "ytd activity count");
    assert!(
        (ytd.total_distance - 1_400_000.0).abs() < f64::EPSILON,
        "ytd distance is ride+run current year, got {}",
        ytd.total_distance
    );
    assert!(
        ytd.total_distance < stats.total_distance,
        "annual distance must not equal lifetime distance (the reported bug)"
    );
}
