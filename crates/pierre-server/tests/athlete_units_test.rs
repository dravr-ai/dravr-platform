// ABOUTME: GET and PUT /api/me/units — metric by default, the Settings choice that changes it, one user per row
// ABOUTME: Pins that nothing but that choice (no provider, no device locale) puts an athlete on miles (carnet#1044)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Athlete units suite (carnet#835, carnet#1044).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use pierre_chat_pipeline::stages::units::append_units_context;
use pierre_core::models::{UnitSystem, User};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
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

#[tokio::test]
async fn an_athlete_who_never_chose_reads_metric_whatever_the_device_says() {
    let resources = common::create_test_server_resources().await.unwrap();
    let user = athlete(&resources).await;
    let token = common::generate_test_token(&resources, &user).await;

    let (status, body) = call(&resources, Method::GET, "/api/me/units", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "units": "metric" }));

    // A US-English device does not put the athlete on miles.
    let (status, body) = call(
        &resources,
        Method::GET,
        "/api/me/units?device_locale=en-US",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "units": "metric" }));
}

#[tokio::test]
async fn a_settings_choice_switches_the_athlete_and_no_one_else() {
    let resources = common::create_test_server_resources().await.unwrap();
    let user = athlete(&resources).await;
    let other = athlete(&resources).await;
    let token = common::generate_test_token(&resources, &user).await;
    let other_token = common::generate_test_token(&resources, &other).await;

    let (status, body) = call(
        &resources,
        Method::PUT,
        "/api/me/units",
        Some(&token),
        Some(json!({ "units": "imperial" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "units": "imperial" }));
    let (_, body) = call(&resources, Method::GET, "/api/me/units", Some(&token), None).await;
    assert_eq!(body, json!({ "units": "imperial" }));

    // Another athlete is untouched.
    let (_, body) = call(
        &resources,
        Method::GET,
        "/api/me/units",
        Some(&other_token),
        None,
    )
    .await;
    assert_eq!(body, json!({ "units": "metric" }));

    // Back to metric.
    let (status, body) = call(
        &resources,
        Method::PUT,
        "/api/me/units",
        Some(&token),
        Some(json!({ "units": "metric" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "units": "metric" }));
}

#[tokio::test]
async fn anything_but_metric_or_imperial_is_refused() {
    let resources = common::create_test_server_resources().await.unwrap();
    let user = athlete(&resources).await;
    let token = common::generate_test_token(&resources, &user).await;

    for body in [
        json!({ "units": "automatic" }),
        json!({ "units": "furlongs" }),
        json!({ "preference": "imperial" }),
        json!({}),
    ] {
        let (status, _) = call(
            &resources,
            Method::PUT,
            "/api/me/units",
            Some(&token),
            Some(body.clone()),
        )
        .await;
        assert!(status.is_client_error(), "{body}: {status}");
    }

    // Nothing was stored by any refusal.
    let (_, body) = call(&resources, Method::GET, "/api/me/units", Some(&token), None).await;
    assert_eq!(body, json!({ "units": "metric" }));
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
        Some(json!({ "units": "metric" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_agents_prompt_names_the_readers_units() {
    let resources = common::create_test_server_resources().await.unwrap();
    let user = athlete(&resources).await;
    let repos = &resources.common.repos;

    // Nothing chosen: metric, no conversion asked for.
    let prompt = append_units_context(repos, user.id, false, "BASE".to_owned()).await;
    assert!(prompt.starts_with("BASE\n\n## Units\n\n"), "{prompt}");
    assert!(prompt.contains("kilometres"), "{prompt}");
    assert!(!prompt.contains("miles"), "{prompt}");

    // The athlete chose imperial: the agent writes miles and feet.
    repos
        .unit_preferences
        .set_units(user.id, UnitSystem::Imperial)
        .await
        .unwrap();
    let prompt = append_units_context(repos, user.id, false, String::new()).await;
    assert!(prompt.contains("miles"), "{prompt}");
    assert!(prompt.contains("feet"), "{prompt}");
}

#[tokio::test]
async fn a_group_scoped_reply_stays_metric_whatever_the_sender_reads() {
    let resources = common::create_test_server_resources().await.unwrap();
    let user = athlete(&resources).await;
    let repos = &resources.common.repos;
    repos
        .unit_preferences
        .set_units(user.id, UnitSystem::Imperial)
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
