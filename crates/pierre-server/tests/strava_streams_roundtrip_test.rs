// ABOUTME: Streams round trip — get_activity_with_streams attaches real Strava per-second samples
// ABOUTME: Pins null handling, GPS dropout dropping, the distance channel, and that the detail tier never pays for streams
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "provider-strava")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once};

use axum::extract::Query;
use axum::http::StatusCode;
use axum::{routing::get, Json, Router};
use chrono::Utc;
use pierre_config::environment::HttpClientConfig;
use pierre_mcp_server::constants::init_server_config;
use pierre_mcp_server::utils::http_client::initialize_http_clients;
use pierre_providers::core::{FitnessProvider, OAuth2Credentials, ProviderConfig};
use pierre_providers::strava_provider::StravaProvider;
use serde_json::{json, Value};
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

/// A detail payload for one ride, minimal but real.
fn detail_payload() -> Value {
    json!({
        "id": 4242,
        "name": "Streams ride",
        "type": "Ride",
        "sport_type": "Ride",
        "start_date": "2026-08-30T10:00:00Z",
        "elapsed_time": 5,
        "moving_time": 5,
        "distance": 100.0,
        "total_elevation_gain": 3.0
    })
}

/// A keyed stream set with a watts dropout (null → 0) and a GPS dropout
/// (null → dropped from the track, never a (0,0) coordinate).
fn streams_payload() -> Value {
    json!({
        "time": { "data": [0, 1, 2, 3, 4] },
        "heartrate": { "data": [120, 121, 122, 123, 124] },
        "watts": { "data": [200, null, 210, 215, 220] },
        "velocity_smooth": { "data": [5.0, 5.1, 5.2, 5.3, 5.4] },
        "altitude": { "data": [10.0, 10.5, 11.0, 11.5, 12.0] },
        "latlng": { "data": [[45.5, -73.6], [45.501, -73.601], null, [45.503, -73.603], [45.504, -73.604]] }
    })
}

/// A run's keyed stream set as Strava sends it: one sample a second, the
/// cumulative `distance` channel in metres beside `time`, and one distance
/// dropout (`null`) while the watch lost the footpod.
fn run_streams_payload() -> Value {
    json!({
        "time": { "data": [0, 1, 2, 3, 4, 5] },
        "distance": { "data": [0.0, 3.2, 6.5, null, 13.1, 16.4] },
        "heartrate": { "data": [140, 142, 145, 147, 150, 151] },
        "velocity_smooth": { "data": [0.0, 3.2, 3.3, 3.3, 3.3, 3.3] },
        "latlng": { "data": [[45.5, -73.6], [45.50003, -73.6], [45.50006, -73.6], [45.50009, -73.6], [45.50012, -73.6], [45.50015, -73.6]] }
    })
}

/// What the mock's streams route saw: how often it was hit and the `keys`
/// each request asked for.
#[derive(Default)]
struct StreamsHits {
    count: AtomicUsize,
    keys: Mutex<Vec<String>>,
}

/// Mock Strava serving the detail endpoint and the streams endpoint, which
/// answers `streams`, or the error status Strava answered with.
async fn provider_serving(
    streams: Result<Value, StatusCode>,
) -> (StravaProvider, Arc<StreamsHits>) {
    let streams_hits = Arc::new(StreamsHits::default());
    let recorder = Arc::clone(&streams_hits);

    let app = Router::new()
        .route("/activities/{id}", get(|| async { Json(detail_payload()) }))
        .route(
            "/activities/{id}/streams",
            get(move |Query(query): Query<HashMap<String, String>>| {
                let recorder = Arc::clone(&recorder);
                let streams = streams.clone();
                async move {
                    recorder.count.fetch_add(1, Ordering::Relaxed);
                    recorder
                        .keys
                        .lock()
                        .unwrap()
                        .push(query.get("keys").cloned().unwrap_or_default());
                    streams.map_or_else(IntoResponse::into_response, |streams| {
                        Json(streams).into_response()
                    })
                }
            }),
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

    (provider, streams_hits)
}

/// Mock Strava serving the ride's stream set, or a 404 on the streams route —
/// Strava's answer for an activity with no samples, such as a manual entry.
async fn provider_with_streams(serve_streams: bool) -> (StravaProvider, Arc<StreamsHits>) {
    provider_serving(if serve_streams {
        Ok(streams_payload())
    } else {
        Err(StatusCode::NOT_FOUND)
    })
    .await
}

use axum::response::IntoResponse;

#[tokio::test]
async fn with_streams_attaches_real_samples_and_handles_dropouts() {
    ensure_http_clients_initialized();
    let (provider, hits) = provider_with_streams(true).await;

    let activity = provider
        .get_activity_with_streams("4242")
        .await
        .expect("activity with streams");

    assert_eq!(
        hits.count.load(Ordering::Relaxed),
        1,
        "one streams round trip"
    );
    let stream = activity
        .time_series_data()
        .expect("streams must be attached");
    assert_eq!(
        stream.timestamps,
        vec![0, 1, 2, 3, 4],
        "Strava's own time stream"
    );
    assert_eq!(
        stream.heart_rate.as_deref(),
        Some(&[120, 121, 122, 123, 124][..])
    );
    assert_eq!(
        stream.power.as_deref(),
        Some(&[200, 0, 210, 215, 220][..]),
        "a watts dropout reads as 0 (no reading), not a hole"
    );
    let gps = stream.gps_coordinates.as_ref().expect("gps track");
    assert_eq!(gps.len(), 4, "the GPS dropout is dropped, never (0,0)");
    assert_eq!(gps[0], (45.5, -73.6));
    assert_eq!(gps[2], (45.503, -73.603));
}

/// Strava's `404` on the streams route is its answer for an activity with no
/// samples — a manual entry. It comes back as a stream set of zero samples
/// with no GPS channel, which a route read settles as "no GPS", never as a
/// read that failed and must be retried.
#[tokio::test]
async fn a_streams_404_is_a_stream_set_of_zero_samples() {
    ensure_http_clients_initialized();
    let (provider, hits) = provider_with_streams(false).await;

    let activity = provider
        .get_activity_with_streams("4242")
        .await
        .expect("activity still served");

    assert_eq!(
        hits.count.load(Ordering::Relaxed),
        1,
        "the streams fetch was tried"
    );
    let streams = activity
        .time_series_data()
        .expect("Strava said the activity has no samples");
    assert!(streams.timestamps.is_empty(), "no fabricated samples");
    assert!(streams.gps_coordinates.is_none(), "no fabricated track");
    assert!(provider.serves_activity_streams());
    assert_eq!(activity.name(), "Streams ride");

    let samples = provider
        .get_activity_streams("4242")
        .await
        .expect("a 404 is an answer, not an error");
    assert!(samples.is_some_and(|s| s.timestamps.is_empty()));
}

/// A streams request Strava could not serve just now carries no stream set:
/// the activity is served without one, which proves nothing about what it
/// recorded.
#[tokio::test]
async fn a_streams_failure_degrades_to_the_plain_activity() {
    ensure_http_clients_initialized();
    let (provider, hits) = provider_serving(Err(StatusCode::SERVICE_UNAVAILABLE)).await;

    let activity = provider
        .get_activity_with_streams("4242")
        .await
        .expect("activity still served");

    assert!(
        hits.count.load(Ordering::Relaxed) >= 1,
        "the streams fetch was tried"
    );
    assert!(
        activity.time_series_data().is_none(),
        "no stream set for a streams request that failed"
    );
    assert_eq!(activity.name(), "Streams ride");
}

#[tokio::test]
async fn the_detail_tier_never_pays_for_streams() {
    ensure_http_clients_initialized();
    let (provider, hits) = provider_with_streams(true).await;

    let activity = provider
        .get_activity_detailed("4242")
        .await
        .expect("detail activity");

    assert_eq!(
        hits.count.load(Ordering::Relaxed),
        0,
        "get_activity_detailed must not hit the streams endpoint — the N+1 \
         detail-promotion path rides on that"
    );
    assert!(activity.time_series_data().is_none());
}

/// A run's cumulative distance reaches the series sample for sample with its
/// time axis, which is what best-effort detection reads; a dropout repeats
/// the last reading instead of falling back to zero.
#[tokio::test]
async fn a_run_stream_fills_the_cumulative_distance_channel() {
    ensure_http_clients_initialized();
    let (provider, hits) = provider_serving(Ok(run_streams_payload())).await;

    let activity = provider
        .get_activity_with_streams("4242")
        .await
        .expect("activity with streams");

    let requested = hits.keys.lock().unwrap().clone();
    assert_eq!(requested.len(), 1, "one streams round trip");
    assert!(
        requested[0].split(',').any(|key| key == "distance"),
        "the distance stream must be asked for, or Strava never sends it: {}",
        requested[0]
    );
    let stream = activity
        .time_series_data()
        .expect("streams must be attached");
    assert_eq!(stream.timestamps, vec![0, 1, 2, 3, 4, 5]);
    assert_eq!(
        stream.distance.as_deref(),
        Some(&[0.0, 3.2, 6.5, 6.5, 13.1, 16.4][..]),
        "metres from the start, aligned with the time axis; the dropout keeps 6.5"
    );
}
