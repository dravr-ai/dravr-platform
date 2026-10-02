// ABOUTME: HTTP tests for PUT /api/me/onboarding/steps/{step_id} — status (incl. not_applicable) + chosen_channel validation
// ABOUTME: Pins the slug guard so only [a-z0-9_] channel slugs (not markup) reach the durable step store
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::json;

use common::{create_test_server_resources, create_test_user, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_mcp_server::routes::onboarding::OnboardingRoutes;

async fn setup() -> (axum::Router, String) {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (_user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let token = generate_test_token(&resources, &user).await;
    let router = OnboardingRoutes::routes(Arc::clone(&resources));
    (router, format!("Bearer {token}"))
}

#[tokio::test]
async fn put_step_accepts_a_valid_channel_slug() {
    let (router, token) = setup().await;
    let resp = AxumTestRequest::put("/api/me/onboarding/steps/messaging_channel")
        .header("Authorization", &token)
        .json(&json!({ "status": "complete", "chosen_channel": "telegram" }))
        .send(router)
        .await;
    assert_eq!(
        resp.status_code(),
        StatusCode::NO_CONTENT,
        "a valid slug ('telegram') must persist"
    );
}

#[tokio::test]
async fn put_step_rejects_a_non_slug_chosen_channel() {
    let (router, token) = setup().await;
    // 25 chars — under the length cap, so this exercises the slug charset guard,
    // not the length guard. Markup must never reach the stored column.
    let resp = AxumTestRequest::put("/api/me/onboarding/steps/messaging_channel")
        .header("Authorization", &token)
        .json(&json!({ "status": "complete", "chosen_channel": "<script>alert(1)</script>" }))
        .send(router)
        .await;
    assert_eq!(
        resp.status_code(),
        StatusCode::BAD_REQUEST,
        "a non-slug chosen_channel (markup) must be rejected, not stored"
    );
}

#[tokio::test]
async fn put_step_rejects_an_unknown_status() {
    let (router, token) = setup().await;
    let resp = AxumTestRequest::put("/api/me/onboarding/steps/profile_type")
        .header("Authorization", &token)
        .json(&json!({ "status": "bogus" }))
        .send(router)
        .await;
    assert_eq!(
        resp.status_code(),
        StatusCode::BAD_REQUEST,
        "an unknown status must be rejected"
    );
}

/// A coach who does not train records the athlete steps as outside their
/// journey; the status round-trips so every surface reads it back.
#[tokio::test]
async fn put_step_accepts_not_applicable_and_status_reads_it_back() {
    let (router, token) = setup().await;
    let resp = AxumTestRequest::put("/api/me/onboarding/steps/parq")
        .header("Authorization", &token)
        .json(&json!({ "status": "not_applicable" }))
        .send(router.clone())
        .await;
    assert_eq!(
        resp.status_code(),
        StatusCode::NO_CONTENT,
        "not_applicable must be an accepted status"
    );

    let status: serde_json::Value = AxumTestRequest::get("/api/me/onboarding-status")
        .header("Authorization", &token)
        .send(router)
        .await
        .assert_status(StatusCode::OK)
        .json();
    let steps = status["steps"].as_array().expect("steps array");
    assert!(
        steps
            .iter()
            .any(|s| s["step_id"] == "parq" && s["status"] == "not_applicable"),
        "the not_applicable parq row must be served back, got {steps:?}"
    );
}
