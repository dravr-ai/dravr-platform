// ABOUTME: Integration tests for admin authentication and authorization system
// ABOUTME: Tests authentication flow, permissions, and token validation using real database connections
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use pierre_core::admin::models::{AdminPermission, CreateAdminTokenRequest};
use pierre_routes_admin::auth::service::AdminAuthService;

#[tokio::test]
async fn test_admin_authentication_flow() {
    // Create test database
    let database = common::create_test_database().await.unwrap();

    // Create JWKS manager for RS256 and generate keys
    let jwks_manager = common::get_shared_test_jwks();

    // Create auth service
    let jwt_secret = "test_jwt_secret_for_admin_auth";
    let repos = database.repositories();
    let auth_service = AdminAuthService::new(
        repos.admin.clone(),
        jwks_manager.clone(),
        AdminAuthService::DEFAULT_CACHE_TTL_SECS,
    );

    // Mint the token through the repository, the same path `pierre-cli token
    // generate` takes, so the stored row is whatever the backend writes.
    let generated = repos
        .admin
        .create_token(
            &CreateAdminTokenRequest {
                service_name: "test_service".to_owned(),
                service_description: None,
                permissions: None,
                expires_in_days: Some(1),
                is_super_admin: false,
                tenant_id: None,
            },
            jwt_secret,
            &*jwks_manager,
        )
        .await
        .unwrap();
    let test_token = generated.jwt_token;

    // Test authentication
    let result = auth_service
        .authenticate_and_authorize(
            &test_token,
            AdminPermission::ProvisionKeys,
            Some("127.0.0.1"),
        )
        .await;

    let validated = match result {
        Ok(v) => v,
        Err(e) => {
            println!("Auth test error: {e}");
            panic!("Expected successful authentication");
        }
    };
    assert_eq!(validated.service_name, "test_service");
    assert!(validated
        .permissions
        .has_permission(&AdminPermission::ProvisionKeys));
}
