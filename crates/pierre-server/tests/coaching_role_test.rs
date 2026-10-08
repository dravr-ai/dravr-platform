// ABOUTME: HTTP integration tests for the coaching role (carnet#827): PUT /api/user/coaching-role and its readers
// ABOUTME: The role answer never sets the reply-style persona; the Coach style still records the role

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The onboarding role answer ("I coach others") used to be written into
//! `coaching_persona` as `coach`, which also held the coach's own training
//! talk to the strict Coach reply contract. These tests pin the split: the
//! role lives in `users.coaches_others`, the style stays where it was, and
//! every reader of the role reads the role.

mod common;
mod helpers;

use common::{create_test_server_resources, create_test_user};
use helpers::axum_test::AxumTestRequest;

use axum::http::StatusCode;
use axum::Router;
use pierre_core::models::CoachingPersona;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::onboarding::OnboardingRoutes;
use pierre_routes_auth::AuthRoutes;
use pierre_services::agents::user_sees_coach_tools;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

async fn setup() -> (Router, String, Uuid, Arc<ServerContext>) {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();
    let token = resources
        .auth
        .auth_manager
        .generate_token(&user, &resources.auth.jwks_manager)
        .unwrap();
    let router = AuthRoutes::routes(resources.auth_routes_context());
    (router, format!("Bearer {token}"), user_id, resources)
}

async fn persona(resources: &ServerContext, user_id: Uuid) -> CoachingPersona {
    resources
        .common
        .repos
        .users
        .get_global(user_id)
        .await
        .unwrap()
        .expect("user row")
        .coaching_persona
}

async fn coaches_others(resources: &ServerContext, user_id: Uuid) -> bool {
    resources
        .common
        .repos
        .users
        .coaches_others(user_id)
        .await
        .unwrap()
}

async fn put_role(router: Router, auth: &str, coaches_others: bool) -> serde_json::Value {
    AxumTestRequest::put("/api/user/coaching-role")
        .header("authorization", auth)
        .json(&json!({ "coaches_others": coaches_others }))
        .send(router)
        .await
        .assert_status(StatusCode::OK)
        .json()
}

#[tokio::test]
async fn the_coach_answer_records_the_role_and_leaves_the_style_alone() {
    let (router, auth, user_id, resources) = setup().await;
    assert!(!coaches_others(&resources, user_id).await);

    let body = put_role(router, &auth, true).await;

    assert_eq!(body["coaches_others"], true);
    assert!(coaches_others(&resources, user_id).await);
    assert_eq!(
        persona(&resources, user_id).await,
        CoachingPersona::Casual,
        "the role answer must not pick the strict Coach reply contract"
    );
}

#[tokio::test]
async fn the_role_answer_keeps_a_style_the_user_chose() {
    let (router, auth, user_id, resources) = setup().await;
    resources
        .common
        .repos
        .users
        .set_coaching_persona(user_id, CoachingPersona::Enthusiast)
        .await
        .unwrap();

    put_role(router, &auth, true).await;

    assert_eq!(
        persona(&resources, user_id).await,
        CoachingPersona::Enthusiast
    );
}

#[tokio::test]
async fn the_athlete_answer_clears_the_role() {
    let (router, auth, user_id, resources) = setup().await;
    put_role(router.clone(), &auth, true).await;

    let body = put_role(router, &auth, false).await;

    assert_eq!(body["coaches_others"], false);
    assert!(!coaches_others(&resources, user_id).await);
}

#[tokio::test]
async fn picking_a_style_in_settings_never_changes_the_role() {
    let (router, auth, user_id, resources) = setup().await;

    // The Coach style is a voice: it grants no role.
    AxumTestRequest::put("/api/user/coaching-persona")
        .header("authorization", &auth)
        .json(&json!({ "persona": "coach" }))
        .send(router.clone())
        .await
        .assert_status(StatusCode::OK);
    assert!(!coaches_others(&resources, user_id).await);
    assert_eq!(persona(&resources, user_id).await, CoachingPersona::Coach);

    // And a coach who picks another style keeps the role.
    put_role(router.clone(), &auth, true).await;
    AxumTestRequest::put("/api/user/coaching-persona")
        .header("authorization", &auth)
        .json(&json!({ "persona": "casual" }))
        .send(router)
        .await
        .assert_status(StatusCode::OK);
    assert!(coaches_others(&resources, user_id).await);
    assert_eq!(persona(&resources, user_id).await, CoachingPersona::Casual);
}

#[tokio::test]
async fn the_role_not_the_style_unlocks_the_coach_tools_and_the_group_step() {
    let (router, auth, user_id, resources) = setup().await;
    let users = resources.common.repos.users.as_ref();

    // The Coach style set directly on the row, without the role, unlocks
    // nothing: the readers read the role.
    users
        .set_coaching_persona(user_id, CoachingPersona::Coach)
        .await
        .unwrap();
    assert!(!user_sees_coach_tools(users, user_id).await);

    put_role(router, &auth, true).await;
    assert!(user_sees_coach_tools(users, user_id).await);

    let status: serde_json::Value = AxumTestRequest::get("/api/me/onboarding-status")
        .header("Authorization", &auth)
        .send(OnboardingRoutes::routes(Arc::clone(&resources)))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(status["coaches_others"], true);
}

#[tokio::test]
async fn the_role_route_requires_a_session() {
    let (router, _auth, _user_id, _resources) = setup().await;

    let response = AxumTestRequest::put("/api/user/coaching-role")
        .json(&json!({ "coaches_others": true }))
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_role_route_rejects_a_body_without_the_answer() {
    let (router, auth, _user_id, _resources) = setup().await;

    let response = AxumTestRequest::put("/api/user/coaching-role")
        .header("authorization", &auth)
        .json(&json!({ "persona": "coach" }))
        .send(router)
        .await;

    assert!(
        response.status_code().is_client_error(),
        "got {}",
        response.status_code()
    );
}
