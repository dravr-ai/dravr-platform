// ABOUTME: Browser device-approval tests — super-admin credentials approve; wrong/non-super-admin creds are rejected
// ABOUTME: Exercises the credential-gated (sudo-style) approval page handlers directly
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::Form;
use chrono::Utc;
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_core::models::{DeviceAuthorization, User};
use pierre_core::permissions::UserRole;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_routes_admin::auth::service::AdminAuthService;
use pierre_routes_admin::handlers::device_web::{
    handle_device_approve_web, handle_device_page, DeviceApproveForm, DevicePageQuery,
};
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit};
use pierre_tool_runtime::guardian::GuardianConfigRegistry;

const TEST_PASSWORD: &str = "Sup3rSecret!";
const USER_CODE: &str = "WDJB-MJHT";

async fn build_context() -> Arc<AdminApiContext> {
    let database = common::create_test_database().await.unwrap();
    let auth_manager = common::create_test_auth_manager();
    let jwks_manager = common::get_shared_test_jwks();

    let database_arc = Arc::new((*database).clone());
    let repos_arc = Arc::new(database_arc.repositories());

    let context = AdminApiContext::new(AdminApiContextInit {
        database: database_arc,
        repos: repos_arc,
        jwt_secret: "test_admin_jwt_secret_for_device_web".to_owned(),
        auth_manager,
        jwks_manager,
        admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
        admin_token_cache_ttl_secs: AdminAuthService::DEFAULT_CACHE_TTL_SECS,
        harness_config_registry: Arc::new(HarnessConfigRegistry::bootstrap()),
        guardian_config_registry: Arc::new(GuardianConfigRegistry::bootstrap()),
        prompt_registry: Arc::new(pierre_contremaitre::PromptRegistry::new()),
        tool_description_registry: Arc::new(pierre_contremaitre::ToolDescriptionRegistry::new()),
        evidence_registry: Arc::new(pierre_contremaitre::EvidenceRegistry::new()),
        messaging_strings_registry: Arc::new(pierre_contremaitre::MessagingStringsRegistry::new()),
        cageux_config_registry: Arc::new(CageuxConfigRegistry::from_env()),
        persona_contract_registry: Arc::new(PersonaContractRegistry::new()),
        contremaitre_config: None,
    });

    Arc::new(context)
}

async fn make_user(context: &AdminApiContext, email: &str, role: UserRole) {
    let mut user = User::new(
        email.to_owned(),
        bcrypt::hash(TEST_PASSWORD, bcrypt::DEFAULT_COST).unwrap(),
        Some("Test".to_owned()),
    );
    user.role = role;
    context.repos.users.create(&user).await.unwrap();
}

async fn create_pending(context: &AdminApiContext) {
    let now = Utc::now().timestamp();
    let record = DeviceAuthorization {
        device_code_hash: "hash-for-web-test".to_owned(),
        user_code: USER_CODE.to_owned(),
        status: "pending".to_owned(),
        approved_by: None,
        created_at: now,
        expires_at: now + 600,
    };
    context
        .repos
        .oauth2_server
        .create_device_authorization(&record)
        .await
        .unwrap();
}

fn approve_form(email: &str, password: &str, action: &str) -> Form<DeviceApproveForm> {
    Form(DeviceApproveForm {
        email: email.to_owned(),
        password: password.to_owned(),
        user_code: USER_CODE.to_owned(),
        action: Some(action.to_owned()),
    })
}

async fn status_of(context: &AdminApiContext) -> String {
    context
        .repos
        .oauth2_server
        .get_device_authorization_by_user_code(USER_CODE)
        .await
        .unwrap()
        .expect("row exists")
        .status
}

#[tokio::test]
async fn page_renders_signin_and_approve_form() {
    let html = handle_device_page(Query(DevicePageQuery {
        user_code: Some(USER_CODE.to_owned()),
    }))
    .await
    .0;
    assert!(html.contains("type=\"password\""), "password field present");
    assert!(html.contains("name=\"email\""), "email field present");
    assert!(html.contains("value=\"approve\""), "approve button present");
    assert!(html.contains(USER_CODE), "user_code shown");
}

#[tokio::test]
async fn super_admin_credentials_approve() {
    let context = build_context().await;
    make_user(&context, "admin@dravr.ai", UserRole::SuperAdmin).await;
    create_pending(&context).await;

    let html = handle_device_approve_web(
        State(context.clone()),
        approve_form("admin@dravr.ai", TEST_PASSWORD, "approve"),
    )
    .await
    .0;
    assert!(html.contains("Login approved"), "success page: {html}");

    let record = context
        .repos
        .oauth2_server
        .get_device_authorization_by_user_code(USER_CODE)
        .await
        .unwrap()
        .expect("row exists");
    assert_eq!(record.status, "approved");
    assert_eq!(record.approved_by.as_deref(), Some("admin@dravr.ai"));
}

#[tokio::test]
async fn deny_marks_denied() {
    let context = build_context().await;
    make_user(&context, "admin@dravr.ai", UserRole::SuperAdmin).await;
    create_pending(&context).await;

    let html = handle_device_approve_web(
        State(context.clone()),
        approve_form("admin@dravr.ai", TEST_PASSWORD, "deny"),
    )
    .await
    .0;
    assert!(html.contains("denied"), "denied page: {html}");
    assert_eq!(status_of(&context).await, "denied");
}

#[tokio::test]
async fn non_super_admin_credentials_cannot_approve() {
    let context = build_context().await;
    make_user(&context, "jf@dravr.ai", UserRole::Admin).await;
    create_pending(&context).await;

    let html = handle_device_approve_web(
        State(context.clone()),
        approve_form("jf@dravr.ai", TEST_PASSWORD, "approve"),
    )
    .await
    .0;
    assert!(
        html.contains("not a super-admin"),
        "rejection message: {html}"
    );
    assert_eq!(status_of(&context).await, "pending", "still pending");
}

#[tokio::test]
async fn wrong_password_cannot_approve() {
    let context = build_context().await;
    make_user(&context, "admin@dravr.ai", UserRole::SuperAdmin).await;
    create_pending(&context).await;

    let html = handle_device_approve_web(
        State(context.clone()),
        approve_form("admin@dravr.ai", "not-the-password", "approve"),
    )
    .await
    .0;
    assert!(
        html.contains("was not recognized"),
        "bad-credential message: {html}"
    );
    assert_eq!(status_of(&context).await, "pending", "still pending");
}
