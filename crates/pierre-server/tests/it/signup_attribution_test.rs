// ABOUTME: Sign-up attribution end to end: the personal tenant and the landing campaign a web sign-up records on its span
// ABOUTME: Then the admin approval names the same tenant, so neither operational event lands on the shared `service` id
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The operational tier of the analytics sink keys an event on its
//! `tenant_id`, and an event without one lands on the single `service`
//! identity. A sign-up raises `user.signed_up` inside the register handler's
//! span, so what that span records is what the event carries:
//!
//! 1. the web register route records the account's personal tenant and the
//!    campaign tags that read as tags, and drops one that is free text;
//! 2. the admin approval names that same personal tenant on `user.approved`.

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::Router;
use helpers::axum_test::AxumTestRequest;
use helpers::notify_capture::{capture_logs_and_spans, capture_notify, only, recorded};
use pierre_core::admin::models::{AdminPermission, CreateAdminTokenRequest};
use pierre_core::models::TenantId;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit, AdminRoutes};
use pierre_routes_auth::AuthRoutes;
use secrecy::ExposeSecret;
use serde_json::{json, Value};
use uuid::Uuid;

/// POST the public register route with `body` and return the new user's id.
async fn register(resources: &ServerContext, body: Value) -> Uuid {
    let response = AxumTestRequest::post("/api/auth/register")
        .json(&body)
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    let status = response.status();
    let body: Value = response.json();
    assert_eq!(status, 201, "registration must succeed: {body}");
    Uuid::parse_str(body["user_id"].as_str().expect("user_id")).expect("user_id is a UUID")
}

/// The tenant the account owns: the personal one registration created.
async fn personal_tenant(resources: &ServerContext, user_id: Uuid) -> TenantId {
    resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()
        .into_iter()
        .find(|tenant| tenant.owner_user_id == user_id)
        .expect("registration gives the account a tenant it owns")
        .id
}

fn admin_api(resources: &Arc<ServerContext>) -> Router {
    AdminRoutes::routes(AdminApiContext::new(AdminApiContextInit {
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
    }))
}

async fn admin_bearer(resources: &Arc<ServerContext>) -> String {
    let token = resources
        .common
        .repos
        .admin
        .create_token(
            &CreateAdminTokenRequest {
                service_name: "signup-attribution".to_owned(),
                service_description: None,
                permissions: Some(vec![AdminPermission::ManageUsers]),
                expires_in_days: Some(1),
                is_super_admin: false,
                tenant_id: None,
                operator_user_id: None,
            },
            resources.auth.admin_jwt_secret.as_ref(),
            &resources.auth.jwks_manager,
        )
        .await
        .unwrap();
    format!("Bearer {}", token.jwt_token.expose_secret())
}

#[tokio::test]
async fn a_web_sign_up_records_its_personal_tenant_and_only_tag_shaped_campaign_tags() {
    let resources = common::create_test_server_resources().await.unwrap();

    let (_, spans, guard) = capture_logs_and_spans();
    let user_id = register(
        &resources,
        json!({
            "email": "campaign-visitor@example.com",
            "password": "securePassword123",
            "utm_source": " newsletter ",
            "utm_medium": "jane@example.com",
            "utm_campaign": "fall-2026",
        }),
    )
    .await;
    drop(guard);

    let tenant = personal_tenant(&resources, user_id).await;
    assert_eq!(
        recorded(&spans, "tenant_id"),
        vec![tenant.to_string()],
        "the sign-up span must carry the personal tenant user.signed_up keys on"
    );
    assert_eq!(
        recorded(&spans, "user_id"),
        vec![user_id.to_string()],
        "the sign-up span names the new account"
    );
    assert_eq!(
        recorded(&spans, "utm_source"),
        vec!["newsletter".to_owned()],
        "a tag-shaped campaign value is recorded, trimmed"
    );
    assert_eq!(
        recorded(&spans, "utm_campaign"),
        vec!["fall-2026".to_owned()],
        "a tag-shaped campaign value is recorded"
    );
    assert!(
        recorded(&spans, "utm_medium").is_empty(),
        "free text in a campaign field never reaches the analytics sink"
    );
}

#[tokio::test]
async fn an_admin_approval_names_the_tenant_the_sign_up_was_attributed_to() {
    let resources = common::create_test_server_resources().await.unwrap();
    let user_id = register(
        &resources,
        json!({ "email": "awaiting-review@example.com", "password": "securePassword123" }),
    )
    .await;
    let tenant = personal_tenant(&resources, user_id).await;
    let bearer = admin_bearer(&resources).await;

    let (events, guard) = capture_notify();
    let response = AxumTestRequest::post(&format!("/admin/approve-user/{user_id}"))
        .header("Authorization", &bearer)
        .json(&json!({ "reason": "attribution test" }))
        .send(admin_api(&resources))
        .await;
    drop(guard);
    assert_eq!(response.status(), 200, "the approval must succeed");

    let approved = only(&events, "user.approved");
    assert_eq!(
        approved.field("tenant_id"),
        tenant.to_string(),
        "user.approved must land on the identity user.signed_up used"
    );
}
