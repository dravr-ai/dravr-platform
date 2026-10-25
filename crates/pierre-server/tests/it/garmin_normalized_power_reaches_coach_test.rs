// ABOUTME: A Garmin activity's normalized power survives the provider read into the coach-facing summary
// ABOUTME: And with it the intensity, efficiency and variability figures the latest snapshot derives from NP
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin NP, provider to coach (carnet#105).
//!
//! `GarminSummary` deserialized `normalizedPower` and then dropped it, so a
//! Garmin ride reached the agent with no NP and `None` for IF / EF / VI. This
//! points the real `GarminProvider` at a local server answering the detail
//! endpoint in Garmin's `summaryDTO` shape and follows the resulting
//! `Activity` into the two places the coach reads it: the `ActivitySummary`
//! the activity tools serialize, and the endurance snapshot's derived factors.
//!
//! `normalizedPower` is the key public Garmin Connect clients read; no live
//! capture confirms it (LIMITATION(registre#105) on that field).
#![cfg(feature = "provider-garmin")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use axum::{routing::get, Json, Router};
use chrono::Utc;
use pierre_core::models::zones::HrZonesUnavailable;
use pierre_fitness_compute::latest_snapshot::build_latest_snapshot;
use pierre_providers::core::{CredentialKind, FitnessProvider, OAuth2Credentials, ProviderConfig};
use pierre_providers::garmin_provider::GarminProvider;
use pierre_tool_runtime::implementations::activity_summary::ActivitySummary;
use secrecy::SecretString;
use serde_json::{json, Value};
use tokio::net::TcpListener;

fn ensure_initialized() {
    common::init_server_config();
    common::init_test_http_clients();
}

/// A cycling activity detail: type under `activityTypeDTO`, metrics under
/// `summaryDTO`, `startTimeGMT` in Garmin's SQL-style spelling.
fn ride_detail() -> Value {
    json!({
        "activityId": 20_260_427_001_u64,
        "activityName": "Tempo ride",
        "activityTypeDTO": { "typeKey": "cycling" },
        "summaryDTO": {
            "startTimeGMT": "2026-04-27 14:25:46.0",
            "distance": 42_000.0,
            "duration": 5_400.0,
            "averageHR": 148.0,
            "averagePower": 214.0,
            "maxPower": 640.0,
            "normalizedPower": 231.0
        }
    })
}

async fn provider_serving(detail: Value) -> GarminProvider {
    let app = Router::new().route(
        "/activity-service/activity/{id}",
        get(move || {
            let detail = detail.clone();
            async move { Json(detail) }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let provider = GarminProvider::with_config(ProviderConfig {
        name: "garmin".to_owned(),
        auth_url: "https://connect.garmin.com/oauth2Confirm".to_owned(),
        token_url: "https://diauth.garmin.com/di-oauth2-service/oauth/token".to_owned(),
        api_base_url: format!("http://{addr}"),
        revoke_url: None,
        default_scopes: Vec::new(),
    });
    provider
        .set_credentials(OAuth2Credentials {
            client_id: "test_client".to_owned(),
            client_secret: SecretString::from("test_secret"),
            access_token: Some(SecretString::from(
                "test_access_token_0123456789abcdef0123456789",
            )),
            refresh_token: Some(SecretString::from("test_refresh_token")),
            expires_at: Some(Utc::now() + chrono::Duration::days(30)),
            scopes: Vec::new(),
            kind: CredentialKind::OAuthBearer,
            request_budget: None,
        })
        .await
        .expect("set_credentials");
    provider
}

#[tokio::test]
async fn a_garmin_rides_np_reaches_the_coach_summary() {
    ensure_initialized();
    let provider = provider_serving(ride_detail()).await;

    let activity = provider
        .get_activity("20260427001")
        .await
        .expect("the detail read succeeds");

    let summary = ActivitySummary::from(&activity);
    assert_eq!(summary.average_power, Some(214));
    assert_eq!(
        summary.normalized_power,
        Some(231),
        "the NP Garmin reported is what the agent reads, not the average and not nothing"
    );
}

#[tokio::test]
async fn a_garmin_rides_np_populates_if_ef_and_vi() {
    ensure_initialized();
    let provider = provider_serving(ride_detail()).await;
    let activity = provider
        .get_activity("20260427001")
        .await
        .expect("the detail read succeeds");

    let snapshot = build_latest_snapshot(
        &[activity],
        7,
        Some(250),
        Err(&HrZonesUnavailable::NoProfile),
    );

    let metrics = snapshot.activities.first().expect("one row").metrics;
    let close = |actual: Option<f64>, expected: f64| {
        let actual = actual.expect("the factor is derived from the NP");
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    };
    close(metrics.intensity_factor, 231.0 / 250.0);
    close(metrics.efficiency_factor, 231.0 / 148.0);
    close(metrics.variability_index, 231.0 / 214.0);
}
