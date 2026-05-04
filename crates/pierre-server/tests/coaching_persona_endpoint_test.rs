// ABOUTME: HTTP integration tests for PUT /api/user/coaching-persona
// ABOUTME: Covers happy path (200 + persistence), bad enum (400), and unauth (401)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! End-to-end tests for the persona update endpoint. We stand up the full
//! [`AuthRoutes`] router with shared `ServerContext`, generate a JWT,
//! and exercise the route through `tower::ServiceExt::oneshot` so the
//! middleware (auth extraction, JSON deserialisation, error envelope)
//! runs exactly as in production.

mod common;
mod helpers;

use common::{create_test_server_resources, create_test_user};
use helpers::axum_test::AxumTestRequest;

use axum::http::StatusCode;
use axum::Router;
use pierre_core::models::CoachingPersona;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::auth::AuthRoutes;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

async fn setup() -> (Router, String, Uuid, Arc<ServerContext>) {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.database).await.unwrap();
    let token = resources
        .auth_manager
        .generate_token(&user, &resources.jwks_manager)
        .unwrap();
    let router = AuthRoutes::routes(resources.clone());
    (router, format!("Bearer {token}"), user_id, resources)
}

#[tokio::test]
async fn put_coaching_persona_persists_power_athlete() {
    let (router, auth, user_id, resources) = setup().await;

    let response = AxumTestRequest::put("/api/user/coaching-persona")
        .header("authorization", &auth)
        .json(&json!({ "persona": "power_athlete" }))
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::OK);
    let body: serde_json::Value = response.json();
    assert_eq!(body["persona"], "power_athlete");

    // Round-trip through the repo to assert the column actually changed.
    let persisted = resources
        .repos
        .users
        .get_global(user_id)
        .await
        .unwrap()
        .expect("user row");
    assert_eq!(persisted.coaching_persona, CoachingPersona::PowerAthlete);
}

#[tokio::test]
async fn put_coaching_persona_accepts_all_four_variants() {
    let (router, auth, _user_id, _resources) = setup().await;

    for variant in ["casual", "enthusiast", "power_athlete", "coach"] {
        let response = AxumTestRequest::put("/api/user/coaching-persona")
            .header("authorization", &auth)
            .json(&json!({ "persona": variant }))
            .send(router.clone())
            .await;
        assert_eq!(
            response.status_code(),
            StatusCode::OK,
            "variant {variant} should be accepted"
        );
        let body: serde_json::Value = response.json();
        assert_eq!(body["persona"], variant);
    }
}

#[tokio::test]
async fn put_coaching_persona_rejects_unknown_value_with_400() {
    let (router, auth, _user_id, _resources) = setup().await;

    let response = AxumTestRequest::put("/api/user/coaching-persona")
        .header("authorization", &auth)
        .json(&json!({ "persona": "elite_pro_max" }))
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn put_coaching_persona_rejects_missing_field_with_400() {
    // serde rejects the body before our handler runs, but the resulting
    // status must still be 4xx — never 200, never 5xx.
    let (router, auth, _user_id, _resources) = setup().await;

    let response = AxumTestRequest::put("/api/user/coaching-persona")
        .header("authorization", &auth)
        .json(&json!({}))
        .send(router)
        .await;

    assert!(
        response.status_code().is_client_error(),
        "missing persona field should be a 4xx, got {}",
        response.status_code()
    );
}

#[tokio::test]
async fn put_coaching_persona_without_auth_rejected() {
    let (router, _auth, _user_id, _resources) = setup().await;

    let response = AxumTestRequest::put("/api/user/coaching-persona")
        .json(&json!({ "persona": "casual" }))
        .send(router)
        .await;

    assert!(
        response.status_code() == StatusCode::UNAUTHORIZED
            || response.status_code() == StatusCode::FORBIDDEN,
        "unauth request should be 401/403, got {}",
        response.status_code()
    );
}
