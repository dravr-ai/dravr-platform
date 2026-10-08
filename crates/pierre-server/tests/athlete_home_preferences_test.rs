// ABOUTME: GET and PUT /api/me/home-preferences — the defaults before any choice, a stored choice, and one user per row
// ABOUTME: Pins that the plan suggestion's dismissal is server-side, so the web and the phone show the same Home

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Athlete Home preferences suite (carnet#820).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use pierre_core::models::User;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

async fn athlete(resources: &Arc<ServerContext>) -> User {
    let email = format!("home-prefs-{}@example.com", Uuid::new_v4());
    let (_, user, _) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .expect("test user");
    user
}

async fn call(
    resources: &Arc<ServerContext>,
    method: Method,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri("/api/me/home-preferences");
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
async fn an_athlete_who_chose_nothing_is_offered_the_plan() {
    let resources = common::create_test_server_resources().await.unwrap();
    let user = athlete(&resources).await;
    let token = common::generate_test_token(&resources, &user).await;

    let (status, body) = call(&resources, Method::GET, Some(&token), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "plan_suggestion_hidden": false }));
}

#[tokio::test]
async fn hiding_the_plan_suggestion_is_stored_and_can_be_undone() {
    let resources = common::create_test_server_resources().await.unwrap();
    let user = athlete(&resources).await;
    let other = athlete(&resources).await;
    let token = common::generate_test_token(&resources, &user).await;
    let other_token = common::generate_test_token(&resources, &other).await;

    let (status, body) = call(
        &resources,
        Method::PUT,
        Some(&token),
        Some(json!({ "plan_suggestion_hidden": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "plan_suggestion_hidden": true }));

    // A later read — the phone's, say — sees the same choice.
    let (_, body) = call(&resources, Method::GET, Some(&token), None).await;
    assert_eq!(body, json!({ "plan_suggestion_hidden": true }));
    // Another athlete's Home is untouched.
    let (_, body) = call(&resources, Method::GET, Some(&other_token), None).await;
    assert_eq!(body, json!({ "plan_suggestion_hidden": false }));

    // Settings brings it back.
    let (status, body) = call(
        &resources,
        Method::PUT,
        Some(&token),
        Some(json!({ "plan_suggestion_hidden": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, body) = call(&resources, Method::GET, Some(&token), None).await;
    assert_eq!(body, json!({ "plan_suggestion_hidden": false }));
}

#[tokio::test]
async fn the_preferences_need_a_signed_in_athlete() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (status, _) = call(&resources, Method::GET, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = call(
        &resources,
        Method::PUT,
        None,
        Some(json!({ "plan_suggestion_hidden": true })),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
