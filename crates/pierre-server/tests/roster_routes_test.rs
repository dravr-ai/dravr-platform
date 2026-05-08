// ABOUTME: HTTP integration tests for /api/roster — gated by users.manages_roster=true
// ABOUTME: Round-trips assign → list → revoke and exercises permission + tenant-isolation 4xx paths
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! End-to-end tests for the coach-athlete roster routes. Stands up the
//! full router via `Router::new().merge(roster::router(...))`, generates
//! a JWT, and exercises the routes with `tower::ServiceExt::oneshot`.
//!
//! Coverage:
//! - Happy path (assign → list → revoke → list-empty)
//! - 403 when caller has `manages_roster=false`
//! - 403 when athlete is in a different tenant
//! - 409 when the same (coach, athlete) is assigned twice
//! - 401 without auth

mod common;
mod helpers;

use common::{create_test_server_resources, create_test_user, create_test_user_with_email};
use helpers::axum_test::AxumTestRequest;

use axum::http::StatusCode;
use axum::Router;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::roster;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

async fn setup_with_coach_and_athlete() -> (Router, String, Uuid, Uuid, Arc<ServerContext>) {
    let resources = create_test_server_resources().await.unwrap();
    let (coach_id, mut coach) = create_test_user(&resources.database).await.unwrap();
    resources
        .repos
        .users
        .set_manages_roster(coach_id, true)
        .await
        .expect("grant manages_roster");
    coach.manages_roster = true;

    let (athlete_id, _athlete) =
        create_test_user_with_email(&resources.database, "athlete@example.com")
            .await
            .unwrap();

    // Both users need to share an active tenant. `create_test_user`
    // gives each user their own tenant — point the athlete's
    // legacy `tenant_id` at the coach's tenant, which also upserts the
    // junction row in `tenant_users` (see `update_user_tenant_id_impl`).
    let coach_tenants = resources
        .repos
        .tenants
        .list_for_user(coach_id)
        .await
        .unwrap();
    let coach_tenant = coach_tenants.first().expect("coach tenant").id;
    resources
        .repos
        .users
        .update_tenant_id(athlete_id, coach_tenant)
        .await
        .expect("link athlete to coach tenant");

    let token = resources
        .auth_manager
        .generate_token_with_tenant(
            &coach,
            &resources.jwks_manager,
            Some(coach_tenant.to_string()),
        )
        .unwrap();
    let router = Router::new().merge(roster::router(resources.clone()));
    let auth_header = format!("Bearer {token}");
    (router, auth_header, coach_id, athlete_id, resources)
}

#[tokio::test]
async fn roster_round_trip_assigns_lists_and_revokes() {
    let (router, auth, coach_id, athlete_id, _resources) = setup_with_coach_and_athlete().await;

    // Assign athlete.
    let assign = AxumTestRequest::post("/api/roster")
        .header("authorization", &auth)
        .json(&json!({ "athlete_user_id": athlete_id }))
        .send(router.clone())
        .await;
    assert_eq!(assign.status_code(), StatusCode::CREATED);
    let body: serde_json::Value = assign.json();
    assert_eq!(body["coach_user_id"], coach_id.to_string());
    assert_eq!(body["athlete_user_id"], athlete_id.to_string());
    assert!(body["revoked_at"].is_null());

    // List shows the athlete.
    let list = AxumTestRequest::get("/api/roster")
        .header("authorization", &auth)
        .send(router.clone())
        .await;
    assert_eq!(list.status_code(), StatusCode::OK);
    let body: serde_json::Value = list.json();
    let assignments = body["assignments"].as_array().expect("assignments array");
    assert_eq!(assignments.len(), 1);
    assert_eq!(assignments[0]["athlete_user_id"], athlete_id.to_string());

    // Revoke.
    let revoke = AxumTestRequest::delete(&format!("/api/roster/{athlete_id}"))
        .header("authorization", &auth)
        .send(router.clone())
        .await;
    assert_eq!(revoke.status_code(), StatusCode::OK);

    // List is empty after revoke.
    let list_after = AxumTestRequest::get("/api/roster")
        .header("authorization", &auth)
        .send(router)
        .await;
    let body: serde_json::Value = list_after.json();
    assert!(body["assignments"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn roster_rejects_caller_without_manages_roster_with_403() {
    // Build a router with a coach whose `manages_roster=false`.
    let resources = create_test_server_resources().await.unwrap();
    let (_user_id, user) = create_test_user(&resources.database).await.unwrap();
    let token = resources
        .auth_manager
        .generate_token(&user, &resources.jwks_manager)
        .unwrap();
    let router = Router::new().merge(roster::router(resources.clone()));
    let auth = format!("Bearer {token}");

    let response = AxumTestRequest::get("/api/roster")
        .header("authorization", &auth)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn roster_rejects_cross_tenant_assignment_with_403() {
    let resources = create_test_server_resources().await.unwrap();
    let (coach_id, mut coach) = create_test_user(&resources.database).await.unwrap();
    resources
        .repos
        .users
        .set_manages_roster(coach_id, true)
        .await
        .expect("grant manages_roster");
    coach.manages_roster = true;
    // Athlete lives in their own tenant — never added to the coach's.
    let (athlete_id, _) = create_test_user_with_email(&resources.database, "outsider@example.com")
        .await
        .unwrap();

    let coach_tenants = resources
        .repos
        .tenants
        .list_for_user(coach_id)
        .await
        .unwrap();
    let coach_tenant = coach_tenants.first().expect("coach tenant").id;
    let token = resources
        .auth_manager
        .generate_token_with_tenant(
            &coach,
            &resources.jwks_manager,
            Some(coach_tenant.to_string()),
        )
        .unwrap();
    let router = Router::new().merge(roster::router(resources.clone()));
    let auth_header = format!("Bearer {token}");

    let response = AxumTestRequest::post("/api/roster")
        .header("authorization", &auth_header)
        .json(&json!({ "athlete_user_id": athlete_id }))
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn roster_double_assign_returns_409() {
    let (router, auth, _coach_id, athlete_id, _resources) = setup_with_coach_and_athlete().await;

    let first = AxumTestRequest::post("/api/roster")
        .header("authorization", &auth)
        .json(&json!({ "athlete_user_id": athlete_id }))
        .send(router.clone())
        .await;
    assert_eq!(first.status_code(), StatusCode::CREATED);

    let second = AxumTestRequest::post("/api/roster")
        .header("authorization", &auth)
        .json(&json!({ "athlete_user_id": athlete_id }))
        .send(router)
        .await;
    assert_eq!(second.status_code(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn roster_without_auth_rejected_401() {
    let resources = create_test_server_resources().await.unwrap();
    let router = Router::new().merge(roster::router(resources));

    let response = AxumTestRequest::get("/api/roster").send(router).await;
    assert!(
        response.status_code() == StatusCode::UNAUTHORIZED
            || response.status_code() == StatusCode::FORBIDDEN,
        "unauth request must be 401/403, got {}",
        response.status_code()
    );
}
