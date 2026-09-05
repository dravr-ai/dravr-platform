// ABOUTME: Full-HTTP e2e chain — register (pending) → approve → login succeeds → onboarding reachable
// ABOUTME: Closes the gap where /register, post-approval login, and onboarding-status are only tested in isolation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! End-to-end HTTP chain for a brand-new user joining the platform.
//!
//! Existing coverage tests these pieces *in isolation*:
//! `routes_auth_http_test::test_public_register_creates_pending_user`
//! (register → pending) and `::test_register_and_login_flow` — but the
//! latter only asserts login "should either succeed or be forbidden due to
//! pending approval", so it never exercises the approved → login transition.
//! `admin_user_approval_e2e_test` covers the real `/admin/approve-user`
//! endpoint, but it creates the user by inserting a `User` struct directly
//! into the DB, bypassing the real `/register` handler.
//!
//! Nothing drives the whole chain through HTTP on one database:
//!
//! 1. `POST /api/auth/register` (public, no admin auth) → user persisted in
//!    `Pending` via the real handler — this is the write path a migration
//!    that drops/renames a `users` column would break, and which the
//!    struct-insert tests cannot catch.
//! 2. Admin approval flips the user to `Active` (state transition only — the
//!    `/admin/approve-user` HTTP endpoint itself is covered by
//!    `admin_user_approval_e2e_test`; re-mounting the heavyweight
//!    `AdminApiContext` here would duplicate it).
//! 3. `POST /oauth/token` (ROPC) now returns a real JWT with `user_status:
//!    active` — the transition the isolation tests punt on.
//! 4. `GET /api/me/onboarding-status` with that JWT is reachable (200),
//!    proving the approved user can enter the onboarding flow.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use helpers::axum_test::AxumTestRequest;
use pierre_config::environment::{
    AppBehaviorConfig, BackupConfig, DatabaseConfig, DatabaseUrl, Environment, SecurityConfig,
    SecurityHeadersConfig, ServerConfig,
};
use pierre_core::models::UserStatus;
use pierre_mcp_server::mcp::resources::{ServerContext, ServerContextOptions};
use pierre_mcp_server::routes::onboarding::OnboardingRoutes;
use pierre_routes_auth::AuthRoutes;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

/// Build a real `ServerContext` over an in-memory `SQLite` DB with public
/// self-registration left in the pending → admin-approval mode (mirrors
/// `routes_auth_http_test::AuthTestSetup::new`).
async fn setup() -> anyhow::Result<Arc<ServerContext>> {
    common::init_server_config();
    let database = common::create_test_database().await?;
    let auth_manager = common::create_test_auth_manager();
    let cache = common::create_test_cache().await?;

    let temp_dir = tempfile::tempdir()?;
    let config = Arc::new(ServerConfig {
        http_port: 8081,
        database: DatabaseConfig {
            url: DatabaseUrl::Memory,
            backup: BackupConfig {
                directory: temp_dir.path().to_path_buf(),
                ..Default::default()
            },
            ..Default::default()
        },
        app_behavior: AppBehaviorConfig {
            ci_mode: true,
            auto_approve_users: false,
            ..Default::default()
        },
        security: SecurityConfig {
            headers: SecurityHeadersConfig {
                environment: Environment::Testing,
            },
            ..Default::default()
        },
        ..Default::default()
    });

    let resources = Arc::new(
        ServerContext::new(
            (*database).clone(),
            (*auth_manager).clone(),
            "test_jwt_secret",
            config,
            cache,
            ServerContextOptions {
                rsa_key_size_bits: Some(2048),
                jwks_manager: Some(common::get_shared_test_jwks()),
                llm_provider: None,
                chat_provider: None,
                extra_tools: Vec::new(),
                billing_provider: None,
                turn_runner: None,
            },
        )
        .await,
    );

    Ok(resources)
}

#[tokio::test]
async fn register_pending_then_approve_then_login_then_onboarding() {
    let resources = setup().await.expect("server context setup failed");
    let auth_routes = AuthRoutes::routes(resources.auth_routes_context());

    let email = "new-joiner@example.com";
    let password = "securePassword123";

    // 1. Public self-registration through the real handler → Pending.
    let register_resp = AxumTestRequest::post("/api/auth/register")
        .json(&json!({
            "email": email,
            "password": password,
            "display_name": "New Joiner"
        }))
        .send(auth_routes.clone())
        .await;
    assert_eq!(
        register_resp.status(),
        201,
        "public registration must succeed with 201 Created"
    );
    let register_body: Value = register_resp.json();
    let user_id = register_body["user_id"]
        .as_str()
        .expect("register response must carry user_id")
        .to_owned();
    assert_eq!(
        register_body["user_status"].as_str(),
        Some("pending"),
        "a self-registered user must start in pending status, got: {register_body}"
    );
    let user_uuid = Uuid::parse_str(&user_id).expect("user_id must be a UUID");

    // 2. Admin approval (state transition; the HTTP approve endpoint itself is
    //    covered by admin_user_approval_e2e_test). A registered account stands
    //    in for the acting admin, because users.approved_by references users.id.
    let approver_resp = AxumTestRequest::post("/api/auth/register")
        .json(&json!({
            "email": "acting-admin@example.com",
            "password": password,
            "display_name": "Acting Admin"
        }))
        .send(auth_routes.clone())
        .await;
    assert_eq!(
        approver_resp.status(),
        201,
        "approver registration must succeed"
    );
    let approver_body: Value = approver_resp.json();
    let approver = Uuid::parse_str(
        approver_body["user_id"]
            .as_str()
            .expect("register response must carry user_id"),
    )
    .expect("approver user_id must be a UUID");
    resources
        .common
        .repos
        .users
        .update_status(user_uuid, UserStatus::Active, Some(approver))
        .await
        .expect("approval state transition must succeed");

    // 3. Login (OAuth2 ROPC, form-encoded) must now succeed with an active user.
    let login_resp = AxumTestRequest::post("/oauth/token")
        .form(&[
            ("grant_type", "password"),
            ("username", email),
            ("password", password),
        ])
        .send(auth_routes.clone())
        .await;
    assert_eq!(
        login_resp.status(),
        200,
        "approved user must be able to log in"
    );
    let login_body: Value = login_resp.json();
    let access_token = login_body["access_token"]
        .as_str()
        .expect("login response must carry an access_token")
        .to_owned();
    assert_eq!(
        login_body["token_type"].as_str(),
        Some("Bearer"),
        "token_type must be Bearer"
    );
    assert_eq!(
        login_body["user"]["user_status"].as_str(),
        Some("active"),
        "post-approval login must report active status, got: {login_body}"
    );

    // 4. The approved user can reach the onboarding gate (the cheap call web +
    //    mobile use to route a fresh user into onboarding).
    let onboarding_routes = OnboardingRoutes::routes(Arc::clone(&resources));
    let onboarding_resp = AxumTestRequest::get("/api/me/onboarding-status")
        .header("Authorization", &format!("Bearer {access_token}"))
        .send(onboarding_routes)
        .await;
    assert_eq!(
        onboarding_resp.status(),
        200,
        "approved user must reach onboarding-status with a valid JWT"
    );
    let onboarding_body: Value = onboarding_resp.json();
    assert!(
        onboarding_body["needs_provider_connection"].is_boolean(),
        "onboarding-status must return needs_provider_connection, got: {onboarding_body}"
    );
    // A freshly approved user has no provider connected yet, so the gate must
    // route them into onboarding.
    assert_eq!(
        onboarding_body["needs_provider_connection"].as_bool(),
        Some(true),
        "a fresh approved user with no provider must be gated into onboarding"
    );
}
