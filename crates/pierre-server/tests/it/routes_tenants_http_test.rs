// ABOUTME: HTTP integration tests for the tenant switch route
// ABOUTME: Tests the switch endpoint with authentication, membership, and error handling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(clippy::uninlined_format_args)]

//! HTTP integration tests for the tenant switch route
//!
//! This test suite validates that the switch endpoint is correctly registered
//! in the router and handles HTTP requests appropriately.

use crate::common;
use crate::helpers;

use dravr_tronc::dates::Timestamp;
use helpers::axum_test::AxumTestRequest;
use pierre_config::environment::{
    AppBehaviorConfig, BackupConfig, DatabaseConfig, DatabaseUrl, Environment, SecurityConfig,
    SecurityHeadersConfig, ServerConfig,
};
use pierre_core::models::{Tenant, TenantId, User};
use pierre_mcp_server::{
    mcp::resources::{ServerContext, ServerContextOptions},
    routes::tenants::TenantRoutes,
};
use serde_json::json;
use std::sync::Arc;

/// Test setup helper for tenant route testing
struct TenantTestSetup {
    resources: Arc<ServerContext>,
    user_id: uuid::Uuid,
    jwt_token: String,
}

impl TenantTestSetup {
    async fn new() -> anyhow::Result<Self> {
        common::init_server_config();
        let database = common::create_test_database().await?;
        let auth_manager = common::create_test_auth_manager();
        let cache = common::create_test_cache().await?;

        // Create test user
        let (user_id, user) = common::create_test_user(&database).await?;

        // Create ServerContext
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
                common::offline_config(config),
                cache,
                ServerContextOptions {
                    rsa_key_size_bits: Some(2048),
                    jwks_manager: Some(common::get_shared_test_jwks()),
                    llm_provider: None,
                    chat_provider: None,
                    extra_tools: Vec::new(),
                    billing_provider: None,
                    turn_runner: None,
                    archive_imports: None,
                    live_records: None,
                    blobs: None,
                    lab_reader: None,
                    pdf_text_bin: None,
                    #[cfg(feature = "client-messaging")]
                    channel_adapters: common::offline_adapters(),
                },
            )
            .await
            .unwrap(),
        );

        // Generate JWT token for the user
        let jwt_token = auth_manager
            .generate_token(&user, &resources.auth.jwks_manager)
            .map_err(|e| anyhow::anyhow!("Failed to generate JWT: {}", e))?;

        Ok(Self {
            resources,
            user_id,
            jwt_token,
        })
    }

    fn routes(&self) -> axum::Router {
        TenantRoutes::routes(self.resources.clone())
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.jwt_token)
    }
}

// ============================================================================
// POST /tenants/switch - Tenant Switching Tests (ASY-176)
// ============================================================================

#[tokio::test]
async fn test_switch_tenant_success() {
    let setup = TenantTestSetup::new().await.expect("Setup failed");

    // Create a tenant and add user as member
    let tenant = Tenant {
        id: TenantId::generate(),
        name: "Switch Target Tenant".to_owned(),
        slug: "switch-target".to_owned(),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: setup.user_id,
        created_at: Timestamp::now(),
        updated_at: Timestamp::now(),
    };

    setup
        .resources
        .common
        .repos
        .tenants
        .create(&tenant)
        .await
        .expect("Failed to create tenant");

    let routes = setup.routes();

    // Switch to the tenant
    let request_body = json!({
        "tenant_id": tenant.id.to_string()
    });

    let response = AxumTestRequest::post("/tenants/switch")
        .header("authorization", &setup.auth_header())
        .json(&request_body)
        .send(routes)
        .await;

    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json();
    assert!(body["token"].is_string());
    assert_eq!(body["active_tenant_id"], tenant.id.to_string());
    assert_eq!(body["tenant_name"], "Switch Target Tenant");
    assert!(body["role"].is_string());
    assert!(body["expires_in"].is_number());
}

#[tokio::test]
async fn test_switch_tenant_no_auth() {
    let setup = TenantTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let request_body = json!({
        "tenant_id": uuid::Uuid::new_v4().to_string()
    });

    let response = AxumTestRequest::post("/tenants/switch")
        .json(&request_body)
        .send(routes)
        .await;

    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_switch_tenant_invalid_auth() {
    let setup = TenantTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let request_body = json!({
        "tenant_id": uuid::Uuid::new_v4().to_string()
    });

    let response = AxumTestRequest::post("/tenants/switch")
        .header("authorization", "Bearer invalid_token_here")
        .json(&request_body)
        .send(routes)
        .await;

    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_switch_tenant_invalid_tenant_id_format() {
    let setup = TenantTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let request_body = json!({
        "tenant_id": "not-a-valid-uuid"
    });

    let response = AxumTestRequest::post("/tenants/switch")
        .header("authorization", &setup.auth_header())
        .json(&request_body)
        .send(routes)
        .await;

    // Should return 400 for invalid input
    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn test_switch_tenant_non_member() {
    let setup = TenantTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    // Create another user first (to satisfy foreign key constraint)
    let other_user = User::new(
        "other@example.com".to_owned(),
        "password_hash".to_owned(),
        Some("Other User".to_owned()),
    );
    setup
        .resources
        .common
        .repos
        .users
        .create(&other_user)
        .await
        .expect("Failed to create other user");

    // Create a tenant owned by the other user
    // create_tenant() auto-adds owner_user_id to tenant_users as owner
    // Our test user is NOT added to tenant_users
    let tenant = Tenant {
        id: TenantId::generate(),
        name: "Other User Tenant".to_owned(),
        slug: "other-user-tenant".to_owned(),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: other_user.id,
        created_at: Timestamp::now(),
        updated_at: Timestamp::now(),
    };

    setup
        .resources
        .common
        .repos
        .tenants
        .create(&tenant)
        .await
        .expect("Failed to create tenant");

    // Try to switch to tenant that test user doesn't belong to
    let request_body = json!({
        "tenant_id": tenant.id.to_string()
    });

    let response = AxumTestRequest::post("/tenants/switch")
        .header("authorization", &setup.auth_header())
        .json(&request_body)
        .send(routes)
        .await;

    // Should return 401 (auth invalid - not a member)
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_switch_tenant_nonexistent() {
    let setup = TenantTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    // Try to switch to a tenant that doesn't exist
    let request_body = json!({
        "tenant_id": uuid::Uuid::new_v4().to_string()
    });

    let response = AxumTestRequest::post("/tenants/switch")
        .header("authorization", &setup.auth_header())
        .json(&request_body)
        .send(routes)
        .await;

    // Should return 401 (user not a member of non-existent tenant)
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_switch_between_multiple_tenants() {
    let setup = TenantTestSetup::new().await.expect("Setup failed");

    // Create two tenants owned by test user
    let tenant1 = Tenant {
        id: TenantId::generate(),
        name: "Tenant One".to_owned(),
        slug: "tenant-one".to_owned(),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: setup.user_id,
        created_at: Timestamp::now(),
        updated_at: Timestamp::now(),
    };

    let tenant2 = Tenant {
        id: TenantId::generate(),
        name: "Tenant Two".to_owned(),
        slug: "tenant-two".to_owned(),
        domain: None,
        plan: "professional".to_owned(),
        owner_user_id: setup.user_id,
        created_at: Timestamp::now(),
        updated_at: Timestamp::now(),
    };

    setup
        .resources
        .common
        .repos
        .tenants
        .create(&tenant1)
        .await
        .expect("Failed to create tenant1");

    setup
        .resources
        .common
        .repos
        .tenants
        .create(&tenant2)
        .await
        .expect("Failed to create tenant2");

    let routes = setup.routes();

    // Switch to tenant1
    let request1 = json!({ "tenant_id": tenant1.id.to_string() });
    let response1 = AxumTestRequest::post("/tenants/switch")
        .header("authorization", &setup.auth_header())
        .json(&request1)
        .send(routes.clone())
        .await;

    assert_eq!(response1.status(), 200);
    let body1: serde_json::Value = response1.json();
    assert_eq!(body1["active_tenant_id"], tenant1.id.to_string());

    // Switch to tenant2
    let request2 = json!({ "tenant_id": tenant2.id.to_string() });
    let response2 = AxumTestRequest::post("/tenants/switch")
        .header("authorization", &setup.auth_header())
        .json(&request2)
        .send(routes)
        .await;

    assert_eq!(response2.status(), 200);
    let body2: serde_json::Value = response2.json();
    assert_eq!(body2["active_tenant_id"], tenant2.id.to_string());
    assert_eq!(body2["tenant_name"], "Tenant Two");
}
