// ABOUTME: GET and PUT /api/me/units — the locale default, the provider's setting, the explicit choice that wins, one user per row
// ABOUTME: Pins the resolution order (Settings, then the provider, then the device locale) the web, the phone and the agent share

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Athlete units suite (carnet#835).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use pierre_chat_pipeline::stages::units::append_units_context;
use pierre_core::models::{Athlete, UnitPreference, UnitSystem, User};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use pierre_services::units::record_provider_units;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

async fn athlete(resources: &Arc<ServerContext>) -> User {
    let email = format!("units-{}@example.com", Uuid::new_v4());
    let (_, user, _) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .expect("test user");
    user
}

async fn call(
    resources: &Arc<ServerContext>,
    method: Method,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(uri);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let request = match body {
        Some(body) => request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => request.body(Body::empty()).unwrap(),
    };
    let response = athlete_home_routes()
        .with_state(Arc::clone(resources))
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// The athlete profile a Strava read answers with, its `measurement_preference` mapped.
fn strava_profile(units: UnitSystem) -> Athlete {
    Athlete {
        id: "42".to_owned(),
        username: "runner".to_owned(),
        firstname: None,
        lastname: None,
        profile_picture: None,
        provider: "strava".to_owned(),
        preferred_units: Some(units),
    }
}

#[tokio::test]
async fn with_nothing_known_the_locale_decides_and_a_read_stores_nothing() {
    let resources = common::create_test_server_resources().await.unwrap();
    let user = athlete(&resources).await;
    let token = common::generate_test_token(&resources, &user).await;

    let (status, body) = call(&resources, Method::GET, "/api/me/units", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({
            "preference": "automatic",
            "units": "metric",
            "source": "locale",
            "provider": null,
            "provider_units": null,
            "device_locale": null,
        })
    );

    // A US-English device reads imperial on the very request that reports it…
    let (status, body) = call(
        &resources,
        Method::GET,
        "/api/me/units?device_locale=en-US",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["units"], "imperial");
    assert_eq!(body["source"], "locale");
    // …without the read storing it.
    assert_eq!(body["device_locale"], Value::Null);
}

#[tokio::test]
async fn the_providers_setting_outranks_the_locale_and_an_explicit_choice_outranks_both() {
    let resources = common::create_test_server_resources().await.unwrap();
    let user = athlete(&resources).await;
    let other = athlete(&resources).await;
    let token = common::generate_test_token(&resources, &user).await;
    let other_token = common::generate_test_token(&resources, &other).await;
    let repos = &resources.common.repos;

    // The device reports en-US, stored for the agent.
    let (status, body) = call(
        &resources,
        Method::PUT,
        "/api/me/units",
        Some(&token),
        Some(json!({ "preference": "automatic", "device_locale": "en-US" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["units"], "imperial");
    assert_eq!(body["source"], "locale");
    assert_eq!(body["device_locale"], "en-US");

    // A Strava profile read set to metres: the provider's setting wins over the locale.
    record_provider_units(
        repos.unit_preferences.as_ref(),
        user.id,
        &strava_profile(UnitSystem::Metric),
    )
    .await;
    let (_, body) = call(&resources, Method::GET, "/api/me/units", Some(&token), None).await;
    assert_eq!(body["units"], "metric");
    assert_eq!(body["source"], "provider");
    assert_eq!(body["provider"], "strava");
    assert_eq!(body["provider_units"], "metric");

    // The athlete picks imperial in Settings: their choice wins over both.
    let (status, body) = call(
        &resources,
        Method::PUT,
        "/api/me/units",
        Some(&token),
        Some(json!({ "preference": "imperial" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({
            "preference": "imperial",
            "units": "imperial",
            "source": "override",
            "provider": "strava",
            "provider_units": "metric",
            "device_locale": "en-US",
        })
    );

    // Another device reports its locale alone: the choice made here stands.
    let (status, body) = call(
        &resources,
        Method::PUT,
        "/api/me/units",
        Some(&token),
        Some(json!({ "device_locale": "fr-FR" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["preference"], "imperial");
    assert_eq!(body["units"], "imperial");
    assert_eq!(body["device_locale"], "fr-FR");

    // Back to automatic: the provider's setting decides again.
    let (_, body) = call(
        &resources,
        Method::PUT,
        "/api/me/units",
        Some(&token),
        Some(json!({ "preference": "automatic" })),
    )
    .await;
    assert_eq!(body["units"], "metric");
    assert_eq!(body["source"], "provider");

    // Another athlete is untouched.
    let (_, body) = call(
        &resources,
        Method::GET,
        "/api/me/units",
        Some(&other_token),
        None,
    )
    .await;
    assert_eq!(body["preference"], "automatic");
    assert_eq!(body["source"], "locale");
}

#[tokio::test]
async fn a_profile_without_a_unit_setting_records_nothing() {
    let resources = common::create_test_server_resources().await.unwrap();
    let user = athlete(&resources).await;
    let token = common::generate_test_token(&resources, &user).await;
    let mut profile = strava_profile(UnitSystem::Imperial);
    profile.preferred_units = None;

    record_provider_units(
        resources.common.repos.unit_preferences.as_ref(),
        user.id,
        &profile,
    )
    .await;
    let (_, body) = call(&resources, Method::GET, "/api/me/units", Some(&token), None).await;
    assert_eq!(body["provider"], Value::Null);
    assert_eq!(body["source"], "locale");
}

#[tokio::test]
async fn a_malformed_choice_or_locale_is_refused() {
    let resources = common::create_test_server_resources().await.unwrap();
    let user = athlete(&resources).await;
    let token = common::generate_test_token(&resources, &user).await;

    let (status, _) = call(
        &resources,
        Method::PUT,
        "/api/me/units",
        Some(&token),
        Some(json!({ "preference": "furlongs" })),
    )
    .await;
    assert!(status.is_client_error(), "{status}");

    let (status, _) = call(
        &resources,
        Method::PUT,
        "/api/me/units",
        Some(&token),
        Some(json!({ "preference": "metric", "device_locale": "en US; drop" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = call(
        &resources,
        Method::PUT,
        "/api/me/units",
        Some(&token),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Nothing was stored by any refusal.
    let (_, body) = call(&resources, Method::GET, "/api/me/units", Some(&token), None).await;
    assert_eq!(body["preference"], "automatic");
}

#[tokio::test]
async fn the_units_need_a_signed_in_athlete() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (status, _) = call(&resources, Method::GET, "/api/me/units", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = call(
        &resources,
        Method::PUT,
        "/api/me/units",
        None,
        Some(json!({ "preference": "metric" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_agents_prompt_names_the_readers_units() {
    let resources = common::create_test_server_resources().await.unwrap();
    let user = athlete(&resources).await;
    let repos = &resources.common.repos;

    // Nothing known and a French profile: metric, no conversion asked for.
    let prompt = append_units_context(repos, user.id, false, "BASE".to_owned()).await;
    assert!(prompt.starts_with("BASE\n\n## Units\n\n"), "{prompt}");
    assert!(prompt.contains("kilometres"), "{prompt}");
    assert!(!prompt.contains("miles"), "{prompt}");

    // The Strava profile says feet: the agent writes miles and feet.
    record_provider_units(
        repos.unit_preferences.as_ref(),
        user.id,
        &strava_profile(UnitSystem::Imperial),
    )
    .await;
    let prompt = append_units_context(repos, user.id, false, String::new()).await;
    assert!(prompt.contains("miles"), "{prompt}");
    assert!(prompt.contains("feet"), "{prompt}");

    // The athlete's own choice of metric wins over Strava's.
    repos
        .unit_preferences
        .set_unit_preference(user.id, UnitPreference::Metric)
        .await
        .unwrap();
    let prompt = append_units_context(repos, user.id, false, String::new()).await;
    assert!(prompt.contains("kilometres"), "{prompt}");
    assert!(!prompt.contains("miles"), "{prompt}");
}

#[tokio::test]
async fn a_group_scoped_reply_stays_metric_whatever_the_sender_reads() {
    let resources = common::create_test_server_resources().await.unwrap();
    let user = athlete(&resources).await;
    let repos = &resources.common.repos;
    repos
        .unit_preferences
        .set_unit_preference(user.id, UnitPreference::Imperial)
        .await
        .unwrap();

    // Private: the sender's imperial choice.
    let private = append_units_context(repos, user.id, false, String::new()).await;
    assert!(private.contains("miles"), "{private}");

    // Group-scoped: read by the group, so metric.
    let group = append_units_context(repos, user.id, true, String::new()).await;
    assert!(group.contains("kilometres"), "{group}");
    assert!(!group.contains("miles"), "{group}");
}
