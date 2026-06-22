// ABOUTME: End-to-end test that get_activities fetches, parses, and maps seeded Strava data correctly
// ABOUTME: Stands up a Strava-shaped mock server and drives the real StravaProvider's activity path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Strava `get_activities` seeded-data round-trip suite.
//
// The MCP comprehensive client test proves the activities tool *dispatches*; it
// does not prove the provider parses and maps a real payload correctly. This
// points the real `StravaProvider` at a local mock returning two seeded
// activities and asserts the canonical `Activity` values survive the round-trip
// — distance, name, and, critically, the granular sport type. Strava's
// deprecated `type` field flattens every bike to `Ride`; the provider must read
// the granular `sport_type` ("MountainBikeRide") and resolve it to a distinct
// mountain-bike classification, not the flattened ride.
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

/// Two seeded Strava activities. The mountain-bike ride carries the deprecated
/// `type: "Ride"` alongside the granular `sport_type: "MountainBikeRide"` so a
/// provider that reads the wrong field is detectable: it would classify the VTT
/// ride as a plain `ride`.
fn activities_payload() -> Value {
    json!([
        {
            "id": 1001,
            "name": "VTT Singletrack",
            "type": "Ride",
            "sport_type": "MountainBikeRide",
            "start_date": "2026-05-20T08:00:00Z",
            "distance": 25_000.0,
            "elapsed_time": 5_400,
            "total_elevation_gain": 800.0
        },
        {
            "id": 1002,
            "name": "Morning Run",
            "type": "Run",
            "sport_type": "Run",
            "start_date": "2026-05-15T06:30:00Z",
            "distance": 10_000.0,
            "elapsed_time": 3_000,
            "total_elevation_gain": 50.0
        }
    ])
}

#[tokio::test]
async fn get_activities_round_trips_seeded_data_with_granular_sport_type() {
    ensure_http_clients_initialized();

    // get_activities for a within-page limit hits a single `/athlete/activities`
    // fetch; the mock ignores the per_page/before query string and returns the
    // fixed seed. No `/athlete` lookup is needed on the activities path.
    let app = Router::new().route(
        "/athlete/activities",
        get(|| async { Json(activities_payload()) }),
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

    let activities = provider
        .get_activities(Some(10), None)
        .await
        .expect("get_activities succeeds");

    assert_eq!(activities.len(), 2, "both seeded activities round-trip");

    let mtb = activities
        .iter()
        .find(|a| a.id() == "1001")
        .expect("mountain-bike activity present");
    let run = activities
        .iter()
        .find(|a| a.id() == "1002")
        .expect("run activity present");

    // Granular sport type is the whole point: the raw label is preserved on
    // `sport_type_detail`, and the resolved `SportType` serializes snake_case so
    // MountainBike -> "mountain_bike". A provider reading the deprecated `type`
    // ("Ride") would instead yield "ride" — this asserts the granular path.
    assert_eq!(
        mtb.sport_type_detail(),
        Some("MountainBikeRide"),
        "raw granular label is preserved for the coach"
    );
    let mtb_json = serde_json::to_value(mtb).expect("serialize mtb activity");
    assert_eq!(
        mtb_json["sport_type"],
        json!("mountain_bike"),
        "VTT must classify as mountain_bike, not the flattened ride"
    );
    assert_eq!(
        mtb_json["distance_meters"].as_f64(),
        Some(25_000.0),
        "MTB distance survives the round-trip"
    );
    assert_eq!(mtb.name(), "VTT Singletrack");

    let run_json = serde_json::to_value(run).expect("serialize run activity");
    assert_eq!(
        run_json["sport_type"],
        json!("run"),
        "run classifies as run"
    );
    assert_eq!(
        run_json["distance_meters"].as_f64(),
        Some(10_000.0),
        "run distance survives the round-trip"
    );
    assert_eq!(run.name(), "Morning Run");
}
