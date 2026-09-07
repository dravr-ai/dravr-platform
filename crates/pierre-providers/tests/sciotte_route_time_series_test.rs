// ABOUTME: Pins that a sciotte GPS track survives convert_activity into cageux TimeSeriesData
// ABOUTME: Asserts real coordinates and elevations arrive index-aligned, and a mis-sized series is dropped
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Sciotte route → `TimeSeriesData` conversion contract.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![cfg(feature = "provider-sciotte")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! A scraped Garmin/Strava activity carries its GPS track in
//! `Activity.route`, and route rendering reads geometry from cageux's
//! `TimeSeriesData` — `gps_coordinates` plus `altitude`. This binary drives the
//! provider against a loopback scraper stub and pins the boundary between the
//! two: the coordinate list arrives verbatim, the elevation series arrives
//! index-aligned as `f32`, and an elevation series whose length disagrees with
//! the track is dropped rather than passed on misaligned.
//!
//! Both scenarios live in one test because they share the process-wide
//! `DRAVR_SCIOTTE_REMOTE_URL`; separate `#[test]`s in this binary would race.

use std::env;

use chrono::{TimeZone, Utc};
use dravr_sciotte::models::AuthSession;
use pierre_providers::core::{FitnessProvider, OAuth2Credentials, ProviderConfig, ProviderFactory};
use pierre_providers::sciotte_provider::SciotteProviderFactory;
use pierre_providers::sciotte_remote::{ENV_AUDIENCE, ENV_REMOTE_URL};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// The track the stub reports for the well-formed activity: four points up a
/// Charlevoix ridge, with elevation and cumulative distance for each.
const TRACK: [(f64, f64); 4] = [
    (47.4212, -70.5109),
    (47.4225, -70.5093),
    (47.4241, -70.5078),
    (47.4256, -70.5061),
];
/// Elevations index-aligned with [`TRACK`].
const ELEVATIONS: [f64; 4] = [312.5, 344.0, 389.25, 421.75];

/// Serve the scraper endpoints the provider calls on a single-activity fetch:
/// `POST /auth/import-session` re-hydrates the session, then
/// `GET /api/activities/{id}` returns the detail payload. Two activity ids are
/// served — `aligned` carries a well-formed track, `misaligned` carries an
/// elevation series one sample short of its coordinates.
///
/// Answers `connection: close` so each request lands on its own accept, and
/// keeps serving until the test drops the listener with the runtime.
fn spawn_scraper_stub(listener: TcpListener) {
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = vec![0_u8; 8192];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();

            let body = if request.contains("/auth/import-session") {
                r#"{"session_id":"stub-session"}"#.to_owned()
            } else if request.contains("/api/activities/misaligned") {
                activity_json("misaligned", &ELEVATIONS[..3])
            } else {
                activity_json("aligned", &ELEVATIONS)
            };

            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
}

/// A scraper detail payload for `id` whose route carries [`TRACK`] and the
/// given elevation series. Optional fields the scraper omits stay absent, as
/// they do on a real detail extract that found no power or weather.
fn activity_json(id: &str, elevations: &[f64]) -> String {
    let coordinates: Vec<[f64; 2]> = TRACK.iter().map(|(lat, lon)| [*lat, *lon]).collect();
    serde_json::json!({
        "id": id,
        "name": "Ridge repeats",
        "sport_type": "trail_running",
        "start_date": "2026-09-05T13:04:00Z",
        "duration_seconds": 3_142,
        "distance_meters": 8_450.0,
        "elevation_gain": 421.0,
        "provider": "strava-scraper",
        "route": {
            "coordinates": coordinates,
            "altitudes_meters": elevations,
            "distances_meters": [0.0, 210.5, 455.0, 690.25],
            "bounds": {
                "min_latitude": 47.4212,
                "max_latitude": 47.4256,
                "min_longitude": -70.5109,
                "max_longitude": -70.5061
            }
        }
    })
    .to_string()
}

/// A provider holding a live session, so every fetch reaches the stub instead
/// of short-circuiting on `provider_auth_required`.
async fn connected_provider() -> Box<dyn FitnessProvider> {
    let config = ProviderConfig {
        name: "sciotte_garmin".to_owned(),
        auth_url: String::new(),
        token_url: String::new(),
        api_base_url: String::new(),
        revoke_url: None,
        default_scopes: vec![],
    };
    let provider = SciotteProviderFactory
        .create(config)
        .expect("sciotte provider construction is infallible"); // Safe: factory returns Ok unconditionally
    let session = AuthSession {
        session_id: "stub-session".to_owned(),
        cookies: vec![],
        created_at: Utc.with_ymd_and_hms(2026, 9, 5, 12, 0, 0).unwrap(), // Safe: literal calendar date is valid
        expires_at: None,
    };
    provider
        .set_credentials(OAuth2Credentials {
            client_id: String::new(),
            client_secret: String::new(),
            access_token: Some(serde_json::to_string(&session).expect("AuthSession serializes")), // Safe: plain data struct, no map keys
            refresh_token: None,
            expires_at: None,
            scopes: vec![],
        })
        .await
        .expect("a serialized session is accepted"); // Safe: the JSON above is a valid AuthSession
    provider
}

#[tokio::test]
async fn a_scraped_gps_track_becomes_index_aligned_time_series_data() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback"); // Safe: ephemeral port on loopback
    let addr = listener
        .local_addr()
        .expect("bound listener has an address"); // Safe: listener is bound
    spawn_scraper_stub(listener);

    env::set_var(ENV_REMOTE_URL, format!("http://{addr}"));
    env::remove_var(ENV_AUDIENCE);

    let provider = connected_provider().await;

    // 1. The well-formed track survives the conversion whole.
    let activity = provider
        .get_activity("aligned")
        .await
        .expect("the stub answers a parseable activity"); // Safe: the stub body is built from the same models

    let series = activity
        .time_series_data()
        .expect("a scraped route must reach cageux TimeSeriesData"); // Safe: asserted behaviour of this test

    let coordinates = series
        .gps_coordinates
        .as_ref()
        .expect("the track populates gps_coordinates"); // Safe: asserted behaviour of this test
    assert_eq!(coordinates.len(), 4, "every scraped point must survive");
    assert_eq!(
        coordinates.as_slice(),
        TRACK.as_slice(),
        "coordinates travel verbatim, in order, as (lat, lon) degrees"
    );

    let altitude = series
        .altitude
        .as_ref()
        .expect("the elevation series populates altitude"); // Safe: asserted behaviour of this test
    assert_eq!(
        altitude.as_slice(),
        &[312.5_f32, 344.0, 389.25, 421.75],
        "elevations narrow to f32 exactly at these magnitudes"
    );

    assert_eq!(
        series.timestamps,
        vec![0_u32, 1, 2, 3],
        "a scraped track has no time axis, so timestamps are sample indices"
    );
    assert!(
        series.heart_rate.is_none()
            && series.power.is_none()
            && series.cadence.is_none()
            && series.speed.is_none()
            && series.temperature.is_none(),
        "a route carries no sensor channels; they stay absent rather than zero-filled"
    );

    // 2. An elevation series that is not index-aligned with the track is
    //    dropped, while the track itself still arrives.
    let misaligned = provider
        .get_activity("misaligned")
        .await
        .expect("the stub answers a parseable activity"); // Safe: the stub body is built from the same models
    let series = misaligned
        .time_series_data()
        .expect("the track survives even when its elevation series does not"); // Safe: asserted behaviour of this test
    assert_eq!(
        series
            .gps_coordinates
            .as_ref()
            .expect("the track populates gps_coordinates") // Safe: asserted behaviour of this test
            .len(),
        4,
        "the coordinate list is unaffected by a bad elevation series"
    );
    assert!(
        series.altitude.is_none(),
        "a 3-sample elevation series over a 4-point track cannot be index-aligned, so it is dropped"
    );

    env::remove_var(ENV_REMOTE_URL);
}
