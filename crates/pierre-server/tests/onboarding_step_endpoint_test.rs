// ABOUTME: HTTP tests for PUT /api/me/onboarding/steps/{step_id} — status (incl. not_applicable) + chosen_channel validation
// ABOUTME: Pins the slug guard, and that the coach-only answer clears the account's own selected agent
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
use pierre_core::models::agents::CreateAgentRequest;
use pierre_core::models::{CoachingPersona, TenantId};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::onboarding::OnboardingRoutes;
use uuid::Uuid;

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

#[tokio::test]
async fn status_reports_an_athlete_with_no_coach_signals() {
    let (router, token) = setup().await;
    let status: serde_json::Value = AxumTestRequest::get("/api/me/onboarding-status")
        .header("Authorization", &token)
        .send(router)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(status["coaches_others"], false);
}

#[tokio::test]
async fn status_reports_a_coach_and_accepts_the_group_step() {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let repos = resources.agent.database.repositories();
    repos
        .users
        .set_coaching_persona(user_id, CoachingPersona::Coach)
        .await
        .unwrap();
    let token = format!("Bearer {}", generate_test_token(&resources, &user).await);
    let router = OnboardingRoutes::routes(Arc::clone(&resources));

    let resp = AxumTestRequest::put("/api/me/onboarding/steps/coach_group")
        .header("Authorization", &token)
        .json(&json!({ "status": "complete" }))
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::NO_CONTENT);

    let status: serde_json::Value = AxumTestRequest::get("/api/me/onboarding-status")
        .header("Authorization", &token)
        .send(router)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(status["coaches_others"], true);
    let steps = status["steps"].as_array().expect("steps array");
    assert!(steps
        .iter()
        .any(|s| s["step_id"] == "coach_group" && s["status"] == "complete"));
}

/// A user holding a selected agent in their tenant, with a token for that
/// tenant: the state signup leaves every new account in.
async fn user_with_selected_agent() -> (Arc<ServerContext>, axum::Router, String, Uuid, TenantId) {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let repos = resources.agent.database.repositories();
    let tenant = repos.tenants.list_for_user(user_id).await.unwrap()[0].id;
    let request: CreateAgentRequest = serde_json::from_value(
        json!({"title":"Endurance","system_prompt":"Test.","category":"training","tags":["run"]}),
    )
    .unwrap();
    let agent = repos
        .agents
        .create(user_id, tenant, &request)
        .await
        .unwrap();
    repos
        .tenants
        .set_selected_agent(tenant, user_id, Some(&agent.id.to_string()))
        .await
        .unwrap();
    let token = format!("Bearer {}", generate_test_token(&resources, &user).await);
    let router = OnboardingRoutes::routes(Arc::clone(&resources));
    (resources, router, token, user_id, tenant)
}

/// The agent signup picked was for an athlete's own training; a coach who does
/// not train has none, so answering that way clears it.
#[tokio::test]
async fn the_coach_only_answer_clears_the_selected_agent() {
    let (resources, router, token, user_id, tenant) = user_with_selected_agent().await;

    let resp = AxumTestRequest::put("/api/me/onboarding/steps/about_you")
        .header("Authorization", &token)
        .json(&json!({ "status": "not_applicable" }))
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::NO_CONTENT);

    let selected = resources
        .agent
        .database
        .repositories()
        .tenants
        .get_selected_agent(tenant, user_id)
        .await
        .unwrap();
    assert_eq!(selected, None, "a coach who does not train holds no agent");
}

#[tokio::test]
async fn an_athletes_answer_keeps_the_selected_agent() {
    let (resources, router, token, user_id, tenant) = user_with_selected_agent().await;
    let tenants = Arc::clone(&resources.agent.database.repositories().tenants);
    let held = tenants.get_selected_agent(tenant, user_id).await.unwrap();
    assert!(held.is_some(), "the fixture selects an agent");

    let resp = AxumTestRequest::put("/api/me/onboarding/steps/parq")
        .header("Authorization", &token)
        .json(&json!({ "status": "complete" }))
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::NO_CONTENT);

    let selected = tenants.get_selected_agent(tenant, user_id).await.unwrap();
    assert_eq!(selected, held, "an athlete keeps the agent they hold");
}
