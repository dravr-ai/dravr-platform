// ABOUTME: Integration tests for admin authentication and authorization system
// ABOUTME: Tests authentication flow, permissions, and token validation using real database connections
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use secrecy::ExposeSecret;

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
    let auth_service = AdminAuthService::new(repos.admin.clone(), jwks_manager.clone());

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
                operator_user_id: None,
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
            test_token.expose_secret(),
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

/// Each authenticated use counts on the token's row, which the console's token
/// list reads as Requests and Last used.
#[tokio::test]
async fn each_authenticated_use_counts_on_the_token_row() {
    let database = common::create_test_database().await.unwrap();
    let jwks_manager = common::get_shared_test_jwks();
    let repos = database.repositories();
    let auth_service = AdminAuthService::new(repos.admin.clone(), jwks_manager.clone());
    let generated = repos
        .admin
        .create_token(
            &CreateAdminTokenRequest {
                service_name: "counted_service".to_owned(),
                service_description: None,
                permissions: None,
                expires_in_days: Some(1),
                is_super_admin: false,
                tenant_id: None,
                operator_user_id: None,
            },
            "test_jwt_secret_for_admin_auth",
            &*jwks_manager,
        )
        .await
        .unwrap();
    let token = generated.jwt_token.expose_secret().to_owned();

    let fresh = repos
        .admin
        .get_token_by_id(&generated.token_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fresh.usage_count, 0);
    assert_eq!(fresh.last_used_at, None);

    // A second of slack: Postgres keeps microseconds, so the stored instant
    // can round below a nanosecond `now`.
    let before = chrono::Utc::now() - chrono::Duration::seconds(1);
    auth_service
        .authenticate(&token, Some("127.0.0.1"))
        .await
        .unwrap();
    auth_service.authenticate(&token, None).await.unwrap();

    let used = repos
        .admin
        .get_token_by_id(&generated.token_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(used.usage_count, 2, "bug: every authenticated use counts");
    assert!(
        used.last_used_at.is_some_and(|at| at >= before),
        "bug: the last use is stamped: {:?}",
        used.last_used_at
    );
    assert_eq!(
        used.last_used_ip.as_deref(),
        Some("127.0.0.1"),
        "a use with no address keeps the last one known"
    );
}
