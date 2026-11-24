// ABOUTME: End-to-end test of the console's user approval workflow over session auth
// ABOUTME: A pending sign-up is listed, approved, then gone from the queue; bad ids and missing auth are refused
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use anyhow::Result;
use common::{create_test_server_resources, generate_test_token};
use dravr_tronc::dates::Timestamp;
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::{User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit, AdminRoutes};
use serde_json::{json, Value};
use serial_test::serial;
use std::sync::Arc;
use uuid::Uuid;

/// The console mount the composition root serves: session auth in front of
/// the admin handlers. A router is consumed by each `send`, so build one per
/// request.
fn console(resources: &Arc<ServerContext>) -> axum::Router {
    let context = AdminApiContext::new(AdminApiContextInit {
        database: resources.agent.database.clone(),
        repos: resources.common.repos.clone(),
        jwt_secret: resources.auth.admin_jwt_secret.to_string(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
        harness_config_registry: resources.fitness.harness_config_registry.clone(),
        guardian_config_registry: resources.fitness.guardian_config_registry.clone(),
        prompt_registry: resources.mcp.prompt_registry.clone(),
        tool_description_registry: resources.mcp.tool_description_registry.clone(),
        evidence_registry: resources.mcp.evidence_registry.clone(),
        messaging_strings_registry: resources.mcp.messaging_strings_registry.clone(),
        cageux_config_registry: resources.fitness.cageux_config_registry.clone(),
        persona_contract_registry: resources.fitness.persona_contract_registry.clone(),
        training_catalogue_registry: resources.mcp.training_catalogue_registry.clone(),
        contremaitre_config: None,
        app_behavior: resources.common.config.app_behavior.clone(),
    });
    AdminRoutes::cookie_admin_routes::<ServerContext>(context, resources)
}

/// An active admin, signed in: `"Bearer <jwt>"`.
async fn admin_session(resources: &Arc<ServerContext>, email: &str) -> Result<String> {
    let mut admin = User::new(
        email.to_owned(),
        "hashed".to_owned(),
        Some("Admin".to_owned()),
    );
    admin.role = UserRole::Admin;
    admin.user_status = UserStatus::Active;
    admin.approved_by = Some(admin.id);
    admin.approved_at = Some(Timestamp::now());
    resources.common.repos.users.create(&admin).await?;
    Ok(format!(
        "Bearer {}",
        generate_test_token(resources, &admin).await
    ))
}

/// Whether the console's pending queue lists `user_id`.
async fn is_pending(resources: &Arc<ServerContext>, auth: &str, user_id: Uuid) -> Result<bool> {
    let response = AxumTestRequest::get("/api/admin/pending-users")
        .header("Authorization", auth)
        .send(console(resources))
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let body: Value = response.json();
    assert_eq!(body["success"], true);
    let users = body["data"]["users"].as_array().expect("a users array");
    assert_eq!(
        body["data"]["count"].as_u64(),
        Some(users.len() as u64),
        "the count is the listing's length"
    );
    Ok(users
        .iter()
        .any(|user| user["id"].as_str() == Some(user_id.to_string().as_str())))
}

/// A sign-up waits in the queue, an admin approves it from the console, and
/// it leaves the queue as an active, attributed account.
#[tokio::test]
#[serial]
async fn a_pending_sign_up_is_listed_approved_and_leaves_the_queue() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let auth = admin_session(&resources, "approval-admin@example.com").await?;

    let mut sign_up = User::new(
        "queued-sign-up@example.com".to_owned(),
        "hashed".to_owned(),
        Some("Queued".to_owned()),
    );
    sign_up.user_status = UserStatus::Pending;
    let user_id = resources.common.repos.users.create(&sign_up).await?;

    assert!(
        is_pending(&resources, &auth, user_id).await?,
        "the sign-up is in the pending queue"
    );

    let response = AxumTestRequest::post(&format!("/api/admin/approve-user/{user_id}"))
        .header("Authorization", &auth)
        .json(&json!({ "reason": "End-to-end test approval" }))
        .send(console(&resources))
        .await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    let body: Value = response.json();
    assert_eq!(body["success"], true);
    assert_eq!(body["data"]["user"]["user_status"], "active");
    assert!(body["data"]["user"]["approved_at"].is_string());

    assert!(
        !is_pending(&resources, &auth, user_id).await?,
        "an approved account is no longer pending"
    );
    let approved = resources
        .common
        .repos
        .users
        .get_global(user_id)
        .await?
        .expect("the account exists");
    assert_eq!(approved.user_status, UserStatus::Active);
    assert!(approved.approved_by.is_some(), "the approval is attributed");
    Ok(())
}

/// A malformed user id is a bad request, and a call without a session never
/// reaches the handler.
#[tokio::test]
#[serial]
async fn the_approval_refuses_a_malformed_id_and_a_missing_session() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let auth = admin_session(&resources, "error-admin@example.com").await?;

    let response = AxumTestRequest::post("/api/admin/approve-user/not-a-uuid")
        .header("Authorization", &auth)
        .json(&json!({ "reason": "Test" }))
        .send(console(&resources))
        .await;
    assert_eq!(response.status(), 400, "{}", response.body_text());

    let response = AxumTestRequest::post(&format!("/api/admin/approve-user/{}", Uuid::new_v4()))
        .json(&json!({ "reason": "Test" }))
        .send(console(&resources))
        .await;
    assert_eq!(response.status(), 401, "{}", response.body_text());

    let response = AxumTestRequest::get("/api/admin/pending-users")
        .send(console(&resources))
        .await;
    assert_eq!(response.status(), 401, "{}", response.body_text());
    Ok(())
}
