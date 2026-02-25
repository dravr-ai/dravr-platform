// ABOUTME: Integration tests for the self-service forgot password flow
// ABOUTME: Tests code generation, anti-enumeration, rate limiting, and full reset cycle
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(clippy::uninlined_format_args)]

//! Integration tests for the self-service password reset flow:
//! 1. User calls POST /api/auth/forgot-password with their email
//! 2. Server generates 6-digit code, stores SHA-256 hash, sends email (if configured)
//! 3. User calls POST /api/auth/complete-reset with code + new password
//! 4. Token is consumed atomically, password updated

mod common;
mod helpers;

use helpers::axum_test::AxumTestRequest;
use pierre_database::plugins::PasswordResetRepository;
use pierre_mcp_server::{
    config::environment::{
        AppBehaviorConfig, BackupConfig, DatabaseConfig, DatabaseUrl, Environment, SecurityConfig,
        SecurityHeadersConfig, ServerConfig,
    },
    mcp::resources::{ServerResources, ServerResourcesOptions},
    routes::auth::AuthRoutes,
};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

/// Test setup helper for forgot password testing
struct ForgotPasswordTestSetup {
    resources: Arc<ServerResources>,
}

impl ForgotPasswordTestSetup {
    async fn new() -> anyhow::Result<Self> {
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
            ServerResources::new(
                (*database).clone(),
                (*auth_manager).clone(),
                "test_jwt_secret",
                config,
                cache,
                ServerResourcesOptions {
                    rsa_key_size_bits: Some(2048),
                    jwks_manager: Some(common::get_shared_test_jwks()),
                    llm_provider: None,
                },
            )
            .await,
        );

        Ok(Self { resources })
    }

    fn routes(&self) -> axum::Router {
        AuthRoutes::routes(self.resources.clone())
    }

    /// Create a test user and return their UUID and email
    async fn create_user(&self) -> anyhow::Result<(uuid::Uuid, String)> {
        let (_, user) = common::create_test_user(&self.resources.database).await?;
        Ok((user.id, user.email))
    }

    /// Create a test user with a specific email
    async fn create_user_with_email(&self, email: &str) -> anyhow::Result<(uuid::Uuid, String)> {
        let (_, user) =
            common::create_test_user_with_email(&self.resources.database, email).await?;
        Ok((user.id, user.email))
    }
}

// ============================================================================
// POST /api/auth/forgot-password - Forgot Password Tests
// ============================================================================

#[tokio::test]
async fn test_forgot_password_existing_email_returns_200() {
    let setup = ForgotPasswordTestSetup::new().await.expect("Setup failed");
    let (_user_id, email) = setup.create_user().await.expect("Failed to create user");
    let routes = setup.routes();

    let response = AxumTestRequest::post("/api/auth/forgot-password")
        .json(&json!({ "email": email }))
        .send(routes)
        .await;

    assert_eq!(
        response.status(),
        200,
        "Forgot password for existing email should return 200"
    );

    let body: serde_json::Value = response.json();
    assert!(
        body["message"].as_str().unwrap().contains("reset code"),
        "Response should contain anti-enumeration message"
    );
}

#[tokio::test]
async fn test_forgot_password_nonexistent_email_returns_200() {
    let setup = ForgotPasswordTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    // Request for an email that doesn't exist — should still return 200
    let response = AxumTestRequest::post("/api/auth/forgot-password")
        .json(&json!({ "email": "nonexistent@example.com" }))
        .send(routes)
        .await;

    assert_eq!(
        response.status(),
        200,
        "Forgot password for nonexistent email should return 200 (anti-enumeration)"
    );

    let body: serde_json::Value = response.json();
    assert!(
        body["message"].as_str().unwrap().contains("reset code"),
        "Response should be identical to existing email case"
    );
}

#[tokio::test]
async fn test_forgot_password_invalid_email_format_returns_400() {
    let setup = ForgotPasswordTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let response = AxumTestRequest::post("/api/auth/forgot-password")
        .json(&json!({ "email": "not-an-email" }))
        .send(routes)
        .await;

    assert_eq!(
        response.status(),
        400,
        "Invalid email format should return 400"
    );
}

#[tokio::test]
async fn test_forgot_password_rate_limiting() {
    let setup = ForgotPasswordTestSetup::new().await.expect("Setup failed");
    let (_user_id, email) = setup
        .create_user_with_email("ratelimit@example.com")
        .await
        .expect("Failed to create user");
    let routes = setup.routes();

    // Send 3 requests (should all succeed)
    for i in 0..3 {
        let response = AxumTestRequest::post("/api/auth/forgot-password")
            .json(&json!({ "email": email }))
            .send(routes.clone())
            .await;

        assert_eq!(response.status(), 200, "Request {} should succeed", i + 1);
    }

    // 4th request should still return 200 (anti-enumeration) but skip silently
    let response = AxumTestRequest::post("/api/auth/forgot-password")
        .json(&json!({ "email": email }))
        .send(routes)
        .await;

    assert_eq!(
        response.status(),
        200,
        "4th request should still return 200 (rate limit is silent)"
    );
}

#[tokio::test]
async fn test_forgot_password_full_flow() {
    use sha2::{Digest, Sha256};

    let setup = ForgotPasswordTestSetup::new().await.expect("Setup failed");
    let (user_id, email) = setup
        .create_user_with_email("fullflow@example.com")
        .await
        .expect("Failed to create user");
    let routes = setup.routes();

    // Step 1: Request forgot password
    let response = AxumTestRequest::post("/api/auth/forgot-password")
        .json(&json!({ "email": email }))
        .send(routes.clone())
        .await;

    assert_eq!(response.status(), 200);

    // Since we can't read the email in tests, we'll simulate by directly creating
    // a known code and testing the complete-reset flow
    let test_code = "123456";
    let code_hash = format!("{:x}", Sha256::digest(test_code.as_bytes()));

    // Store a known token via the repository directly
    setup
        .resources
        .database
        .store_token_with_ttl(user_id, &code_hash, "self_service", 15)
        .await
        .expect("Failed to store test token");

    // Step 2: Complete reset with the known code
    let reset_response = AxumTestRequest::post("/api/auth/complete-reset")
        .json(&json!({
            "reset_token": test_code,
            "new_password": "NewSecurePassword123"
        }))
        .send(routes.clone())
        .await;

    assert_eq!(
        reset_response.status(),
        200,
        "Complete reset with valid code should succeed"
    );

    // Step 3: Verify login with new password works
    let login_request = [
        ("grant_type", "password"),
        ("username", email.as_str()),
        ("password", "NewSecurePassword123"),
    ];

    let login_response = AxumTestRequest::post("/oauth/token")
        .form(&login_request)
        .send(routes.clone())
        .await;

    assert_eq!(
        login_response.status(),
        200,
        "Login with new password should succeed after reset"
    );

    // Step 4: Verify old password no longer works
    let old_login_request = [
        ("grant_type", "password"),
        ("username", email.as_str()),
        ("password", "password123"),
    ];

    let old_login_response = AxumTestRequest::post("/oauth/token")
        .form(&old_login_request)
        .send(routes)
        .await;

    assert_eq!(
        old_login_response.status(),
        400,
        "Login with old password should fail after reset"
    );
}

#[tokio::test]
async fn test_forgot_password_code_single_use() {
    use sha2::{Digest, Sha256};

    let setup = ForgotPasswordTestSetup::new().await.expect("Setup failed");
    let (user_id, _email) = setup
        .create_user_with_email("singleuse@example.com")
        .await
        .expect("Failed to create user");
    let routes = setup.routes();

    // Store a known token
    let test_code = "654321";
    let code_hash = format!("{:x}", Sha256::digest(test_code.as_bytes()));
    setup
        .resources
        .database
        .store_token_with_ttl(user_id, &code_hash, "self_service", 15)
        .await
        .expect("Failed to store test token");

    // First use should succeed
    let first_response = AxumTestRequest::post("/api/auth/complete-reset")
        .json(&json!({
            "reset_token": test_code,
            "new_password": "FirstPassword123"
        }))
        .send(routes.clone())
        .await;

    assert_eq!(first_response.status(), 200, "First use should succeed");

    // Second use should fail (token already consumed)
    let second_response = AxumTestRequest::post("/api/auth/complete-reset")
        .json(&json!({
            "reset_token": test_code,
            "new_password": "SecondPassword456"
        }))
        .send(routes)
        .await;

    assert_eq!(
        second_response.status(),
        404,
        "Second use of same code should fail"
    );
}

#[tokio::test]
async fn test_forgot_password_expired_code_rejected() {
    use sha2::{Digest, Sha256};

    let setup = ForgotPasswordTestSetup::new().await.expect("Setup failed");
    let (user_id, _email) = setup
        .create_user_with_email("expired@example.com")
        .await
        .expect("Failed to create user");
    let routes = setup.routes();

    // Store a token with 0-minute TTL (already expired)
    let test_code = "111222";
    let code_hash = format!("{:x}", Sha256::digest(test_code.as_bytes()));
    setup
        .resources
        .database
        .store_token_with_ttl(user_id, &code_hash, "self_service", 0)
        .await
        .expect("Failed to store test token");

    // Small delay to ensure expiry is in the past
    sleep(Duration::from_millis(100)).await;

    let response = AxumTestRequest::post("/api/auth/complete-reset")
        .json(&json!({
            "reset_token": test_code,
            "new_password": "ShouldNotWork123"
        }))
        .send(routes)
        .await;

    assert_eq!(response.status(), 404, "Expired code should be rejected");
}
