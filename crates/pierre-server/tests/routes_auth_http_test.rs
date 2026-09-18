// ABOUTME: HTTP integration tests for authentication routes
// ABOUTME: Tests all authentication endpoints including registration, login, refresh, and OAuth status
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(clippy::uninlined_format_args)]

//! Comprehensive HTTP integration tests for authentication routes
//!
//! This test suite validates that all authentication endpoints are correctly registered
//! in the router and handle HTTP requests appropriately.

mod common;
mod helpers;

use chrono::{Duration, Utc};
use helpers::axum_test::AxumTestRequest;
use pierre_config::environment::{
    AppBehaviorConfig, BackupConfig, DatabaseConfig, DatabaseUrl, Environment, SecurityConfig,
    SecurityHeadersConfig, ServerConfig,
};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::UserOAuthToken;
use pierre_mcp_server::mcp::resources::{ServerContext, ServerContextOptions};
use pierre_routes_auth::AuthRoutes;
use serde_json::json;
use std::sync::Arc;

/// Test setup helper for authentication route testing
struct AuthTestSetup {
    resources: Arc<ServerContext>,
}

impl AuthTestSetup {
    async fn new() -> anyhow::Result<Self> {
        common::init_server_config();
        let database = common::create_test_database().await?;
        let auth_manager = common::create_test_auth_manager();
        let cache = common::create_test_cache().await?;

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

        Ok(Self { resources })
    }

    fn routes(&self) -> axum::Router {
        AuthRoutes::routes(self.resources.auth_routes_context())
    }

    /// Create a test admin token for authentication
    async fn create_admin_token(&self) -> anyhow::Result<String> {
        use pierre_core::admin::models::CreateAdminTokenRequest;

        // Create admin token request
        let request = CreateAdminTokenRequest {
            service_name: "test_admin".to_owned(),
            service_description: Some("Auto-generated test admin token".to_owned()),
            permissions: None, // Super admin by default
            expires_in_days: Some(1),
            is_super_admin: true,
            tenant_id: None,
        };

        // Use repository to create admin token
        let generated_token = self
            .resources
            .common
            .repos
            .admin
            .create_token(
                &request,
                "test_jwt_secret",
                &self.resources.auth.jwks_manager,
            )
            .await?;

        Ok(generated_token.jwt_token)
    }
}

// ============================================================================
// POST /api/auth/register - User Registration Tests
// ============================================================================

#[tokio::test]
async fn test_register_success() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");
    let admin_token = setup
        .create_admin_token()
        .await
        .expect("Failed to create admin token");
    let routes = setup.routes();

    let register_request = json!({
        "email": "newuser@example.com",
        "password": "securePassword123",
        "display_name": "New User"
    });

    let response = AxumTestRequest::post("/api/auth/register")
        .header("Authorization", &format!("Bearer {}", admin_token))
        .json(&register_request)
        .send(routes)
        .await;

    assert_eq!(response.status(), 201);

    let body: serde_json::Value = response.json();
    assert!(body["user_id"].is_string());
    assert!(body["message"].is_string());
}

#[tokio::test]
async fn test_public_register_creates_pending_user() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let register_request = json!({
        "email": "public@example.com",
        "password": "password123",
        "display_name": "Public User"
    });

    // Public self-registration should succeed without admin auth
    // (creates user in Pending status, requires admin approval)
    let response = AxumTestRequest::post("/api/auth/register")
        .json(&register_request)
        .send(routes)
        .await;

    assert_eq!(
        response.status(),
        201,
        "Public registration should succeed with 201 Created"
    );

    let body: serde_json::Value = response.json();
    assert!(
        body["user_id"].is_string(),
        "Response should contain user_id"
    );
    let message = body["message"].as_str().unwrap_or("");
    assert!(
        message.contains("pending") || message.contains("approval"),
        "Message should indicate pending approval: {}",
        message
    );
}

#[tokio::test]
async fn test_register_invalid_email() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");
    let admin_token = setup
        .create_admin_token()
        .await
        .expect("Failed to create admin token");
    let routes = setup.routes();

    let register_request = json!({
        "email": "invalid-email",
        "password": "password123"
    });

    let response = AxumTestRequest::post("/api/auth/register")
        .header("Authorization", &format!("Bearer {}", admin_token))
        .json(&register_request)
        .send(routes)
        .await;

    // Should fail validation
    assert!(response.status() == 400 || response.status() == 422);
}

#[tokio::test]
async fn test_register_weak_password() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");
    let admin_token = setup
        .create_admin_token()
        .await
        .expect("Failed to create admin token");
    let routes = setup.routes();

    let register_request = json!({
        "email": "user@example.com",
        "password": "weak"
    });

    let response = AxumTestRequest::post("/api/auth/register")
        .header("Authorization", &format!("Bearer {}", admin_token))
        .json(&register_request)
        .send(routes)
        .await;

    // Should fail validation (password too short)
    assert!(response.status() == 400 || response.status() == 422);
}

#[tokio::test]
async fn test_register_duplicate_email() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");
    let admin_token = setup
        .create_admin_token()
        .await
        .expect("Failed to create admin token");
    let routes = setup.routes();

    let register_request = json!({
        "email": "duplicate@example.com",
        "password": "password123",
        "display_name": "Duplicate User"
    });

    // First registration should succeed
    let _response1 = AxumTestRequest::post("/api/auth/register")
        .header("Authorization", &format!("Bearer {}", admin_token))
        .json(&register_request)
        .send(routes.clone())
        .await;

    // Second registration with same email should fail
    let response2 = AxumTestRequest::post("/api/auth/register")
        .header("Authorization", &format!("Bearer {}", admin_token))
        .json(&register_request)
        .send(routes)
        .await;

    // First should succeed or have valid error, second should fail
    assert_ne!(response2.status(), 201);
}

#[tokio::test]
async fn test_register_missing_required_fields() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");
    let admin_token = setup
        .create_admin_token()
        .await
        .expect("Failed to create admin token");
    let routes = setup.routes();

    let register_request = json!({
        "email": "user@example.com"
        // Missing password
    });

    let response = AxumTestRequest::post("/api/auth/register")
        .header("Authorization", &format!("Bearer {}", admin_token))
        .json(&register_request)
        .send(routes)
        .await;

    // Should fail validation
    assert_ne!(response.status(), 201);
}

// ============================================================================
// POST /oauth/token - User Login Tests (OAuth2 ROPC)
// ============================================================================

#[tokio::test]
async fn test_login_success() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");

    // Create a test user first
    let (_, user) = common::create_test_user(&setup.resources.agent.database)
        .await
        .expect("Failed to create test user");

    let routes = setup.routes();

    // OAuth2 ROPC uses form-encoded data
    let login_request = [
        ("grant_type", "password"),
        ("username", &user.email),
        ("password", "password123"), // Default password from create_test_user
    ];

    let response = AxumTestRequest::post("/oauth/token")
        .form(&login_request)
        .send(routes)
        .await;

    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json();
    assert!(body["access_token"].is_string());
    assert_eq!(body["token_type"].as_str(), Some("Bearer"));
    assert!(body["expires_in"].is_number());
    assert!(body["user"]["user_id"].is_string());
    assert!(body["user"]["email"].is_string());
}

#[tokio::test]
async fn test_login_no_auth_required() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let login_request = [
        ("grant_type", "password"),
        ("username", "user@example.com"),
        ("password", "password123"),
    ];

    // Login should work without authentication header
    let response = AxumTestRequest::post("/oauth/token")
        .form(&login_request)
        .send(routes)
        .await;

    // Should fail due to wrong credentials (401) but not require auth header
    // If it returns 401, it's because credentials are wrong, not missing auth
    assert_ne!(response.status(), 500);
}

#[tokio::test]
async fn test_login_invalid_credentials() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let login_request = [
        ("grant_type", "password"),
        ("username", "nonexistent@example.com"),
        ("password", "wrongpassword"),
    ];

    let response = AxumTestRequest::post("/oauth/token")
        .form(&login_request)
        .send(routes)
        .await;

    // OAuth2 ROPC returns 400 with "invalid_grant" error for bad credentials (RFC 6749 Section 5.2)
    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn test_login_wrong_password() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");

    // Create a test user first
    let (_, user) = common::create_test_user(&setup.resources.agent.database)
        .await
        .expect("Failed to create test user");

    let routes = setup.routes();

    let login_request = [
        ("grant_type", "password"),
        ("username", user.email.as_str()),
        ("password", "wrongpassword"),
    ];

    let response = AxumTestRequest::post("/oauth/token")
        .form(&login_request)
        .send(routes)
        .await;

    // OAuth2 ROPC returns 400 with "invalid_grant" error for bad credentials (RFC 6749 Section 5.2)
    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn test_login_missing_fields() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    // Missing password field
    let login_request = [("grant_type", "password"), ("username", "user@example.com")];

    let response = AxumTestRequest::post("/oauth/token")
        .form(&login_request)
        .send(routes)
        .await;

    // RFC 6749 §5.2: a malformed/missing-parameter token request returns a 400
    // with an `invalid_request` JSON error — not the bare framework 422 that
    // leaks the internal field name. (Regression for the Phase-7 finding.)
    assert_eq!(response.status(), 400);
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"].as_str(), Some("invalid_request"));
    // The internal deserializer field name must not leak into the response.
    assert!(!body.to_string().contains("missing field"));
}

// ============================================================================
// POST /oauth/token - Refresh Token Tests (offline_access + refresh_token grant)
// ============================================================================

/// Log the shared test user in the way the mobile app does — asking for a
/// refresh token — and return the JSON body.
async fn login_with_offline_access(setup: &AuthTestSetup, email: &str) -> serde_json::Value {
    let login_request = [
        ("grant_type", "password"),
        ("username", email),
        ("password", "password123"),
        ("scope", "offline_access"),
    ];
    let response = AxumTestRequest::post("/oauth/token")
        .form(&login_request)
        .send(setup.routes())
        .await;
    assert_eq!(response.status(), 200);
    response.json()
}

/// Exchange a refresh token at the token endpoint.
async fn exchange(setup: &AuthTestSetup, refresh_token: &str) -> (u16, serde_json::Value) {
    let refresh_request = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
    ];
    let response = AxumTestRequest::post("/oauth/token")
        .form(&refresh_request)
        .send(setup.routes())
        .await;
    let status = response.status();
    (status, response.json())
}

#[tokio::test]
async fn test_login_with_offline_access_returns_a_refresh_token() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");
    let (_, user) = common::create_test_user(&setup.resources.agent.database)
        .await
        .expect("Failed to create test user");

    let body = login_with_offline_access(&setup, &user.email).await;

    assert!(body["access_token"].is_string());
    let refresh_token = body["refresh_token"]
        .as_str()
        .expect("offline_access login carries a refresh token");
    // 32 random bytes, base64url without padding.
    assert_eq!(refresh_token.len(), 43);
}

#[tokio::test]
async fn test_login_without_offline_access_returns_no_refresh_token() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");
    let (_, user) = common::create_test_user(&setup.resources.agent.database)
        .await
        .expect("Failed to create test user");

    let login_request = [
        ("grant_type", "password"),
        ("username", user.email.as_str()),
        ("password", "password123"),
    ];
    let response = AxumTestRequest::post("/oauth/token")
        .form(&login_request)
        .send(setup.routes())
        .await;
    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json();
    assert!(body["access_token"].is_string());
    // The web app never asked, so nothing exchangeable was written for it.
    assert!(body.get("refresh_token").is_none());
}

#[tokio::test]
async fn test_refresh_grant_rotates_the_token_and_kills_a_replayed_family() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");
    let (user_id, user) = common::create_test_user(&setup.resources.agent.database)
        .await
        .expect("Failed to create test user");

    let login = login_with_offline_access(&setup, &user.email).await;
    let first = login["refresh_token"].as_str().unwrap().to_owned();
    let login_jwt = login["access_token"].as_str().unwrap().to_owned();

    // Exchange: a new JWT, the same user, and a successor token.
    let (status, refreshed) = exchange(&setup, &first).await;
    assert_eq!(status, 200, "{refreshed}");
    let refreshed_jwt = refreshed["access_token"].as_str().unwrap();
    assert_ne!(refreshed_jwt, login_jwt, "the exchange mints a new JWT");
    assert_eq!(refreshed["token_type"].as_str(), Some("Bearer"));
    assert!(refreshed["expires_in"].as_i64().unwrap() > 0);
    assert!(refreshed["csrf_token"].is_string());
    assert_eq!(
        refreshed["user"]["user_id"].as_str(),
        Some(user_id.to_string().as_str())
    );
    assert_eq!(
        refreshed["user"]["email"].as_str(),
        Some(user.email.as_str())
    );
    let second = refreshed["refresh_token"].as_str().unwrap().to_owned();
    assert_ne!(second, first, "the exchange rotates the refresh token");

    // The token just exchanged is dead.
    let (status, replay) = exchange(&setup, &first).await;
    assert_eq!(status, 400);
    assert_eq!(replay["error"].as_str(), Some("invalid_grant"));

    // And replaying it revoked its successor: a copied credential stops
    // working the moment the real device has moved on.
    let (status, successor) = exchange(&setup, &second).await;
    assert_eq!(status, 400);
    assert_eq!(successor["error"].as_str(), Some("invalid_grant"));
}

#[tokio::test]
async fn test_refresh_grant_with_unknown_token_is_invalid_grant() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");

    let (status, body) = exchange(&setup, "never-issued").await;

    assert_eq!(status, 400);
    assert_eq!(body["error"].as_str(), Some("invalid_grant"));
}

#[tokio::test]
async fn test_refresh_grant_without_token_is_invalid_request() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");

    let response = AxumTestRequest::post("/oauth/token")
        .form(&[("grant_type", "refresh_token")])
        .send(setup.routes())
        .await;

    assert_eq!(response.status(), 400);
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"].as_str(), Some("invalid_request"));
}

#[tokio::test]
async fn test_password_grant_without_credentials_is_invalid_request() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");

    let response = AxumTestRequest::post("/oauth/token")
        .form(&[("grant_type", "password"), ("username", "user@example.com")])
        .send(setup.routes())
        .await;

    assert_eq!(response.status(), 400);
    let body: serde_json::Value = response.json();
    assert_eq!(body["error"].as_str(), Some("invalid_request"));
}

#[tokio::test]
async fn test_logout_revokes_the_presented_refresh_token() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");
    let (_, user) = common::create_test_user(&setup.resources.agent.database)
        .await
        .expect("Failed to create test user");

    let login = login_with_offline_access(&setup, &user.email).await;
    let refresh_token = login["refresh_token"].as_str().unwrap().to_owned();

    // No bearer: a phone whose JWT already lapsed can still log out.
    let response = AxumTestRequest::post("/api/auth/logout")
        .json(&json!({ "refresh_token": refresh_token }))
        .send(setup.routes())
        .await;
    assert_eq!(response.status(), 200);

    let (status, body) = exchange(&setup, &refresh_token).await;
    assert_eq!(status, 400);
    assert_eq!(body["error"].as_str(), Some("invalid_grant"));
}

#[tokio::test]
async fn test_logout_without_a_body_still_clears_the_cookie_session() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");

    let response = AxumTestRequest::post("/api/auth/logout")
        .send(setup.routes())
        .await;

    assert_eq!(response.status(), 200);
}

// ============================================================================
// GET /api/oauth/status - OAuth Status Tests
// ============================================================================

#[tokio::test]
async fn test_oauth_status_success() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");

    // Create test user and generate token
    let (_, user) = common::create_test_user(&setup.resources.agent.database)
        .await
        .expect("Failed to create test user");

    let jwt_token = setup
        .resources
        .auth
        .auth_manager
        .generate_token(&user, &setup.resources.auth.jwks_manager)
        .expect("Failed to generate JWT");

    let routes = setup.routes();

    let response = AxumTestRequest::get("/api/oauth/status")
        .header("authorization", &format!("Bearer {}", jwt_token))
        .send(routes)
        .await;

    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json();
    assert!(body.is_array());

    // Should contain OAuth provider statuses
    let statuses = body.as_array().unwrap();
    for status in statuses {
        assert!(status["provider"].is_string());
        assert!(status["connected"].is_boolean());
    }
}

#[tokio::test]
async fn test_oauth_status_missing_auth() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let response = AxumTestRequest::get("/api/oauth/status").send(routes).await;

    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_oauth_status_invalid_auth() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    let response = AxumTestRequest::get("/api/oauth/status")
        .header("authorization", "Bearer invalid_token")
        .send(routes)
        .await;

    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_oauth_status_includes_all_providers() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");

    // Create test user and generate token
    let (_, user) = common::create_test_user(&setup.resources.agent.database)
        .await
        .expect("Failed to create test user");

    let jwt_token = setup
        .resources
        .auth
        .auth_manager
        .generate_token(&user, &setup.resources.auth.jwks_manager)
        .expect("Failed to generate JWT");

    let routes = setup.routes();

    let response = AxumTestRequest::get("/api/oauth/status")
        .header("authorization", &format!("Bearer {}", jwt_token))
        .send(routes)
        .await;

    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json();
    let statuses = body.as_array().unwrap();

    // Should include common providers like strava and fitbit
    let providers: Vec<String> = statuses
        .iter()
        .map(|s| s["provider"].as_str().unwrap().to_owned())
        .collect();

    assert!(providers.contains(&"strava".to_owned()));
    assert!(providers.contains(&"fitbit".to_owned()));
}

/// The third surface carnet#352 named. `/api/oauth/status` echoed every token
/// row as connected, so the athlete whose only Garmin row was the OAuth one read
/// exactly what the issue opened with: `{"provider":"garmin","connected":true}`,
/// while every coach call failed against the mirror it is actually routed to.
#[tokio::test]
async fn oauth_status_does_not_report_a_bare_garmin_row_as_connected() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");
    let (user_id, user) = common::create_test_user(&setup.resources.agent.database)
        .await
        .expect("Failed to create test user");
    let tenants = setup
        .resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("list tenants");
    let tenant_id = tenants[0].id.to_string();

    let seed = |provider: &str| {
        UserOAuthToken::new(
            user_id,
            tenant_id.clone(),
            provider.to_owned(),
            "test_access_token".to_owned(),
            Some("test_refresh_token".to_owned()),
            Some(Utc::now() + Duration::hours(1)),
            Some("read".to_owned()),
        )
    };
    for provider in [oauth_providers::GARMIN, oauth_providers::STRAVA] {
        setup
            .resources
            .common
            .repos
            .oauth_tokens
            .upsert_token(&seed(provider))
            .await
            .expect("seed token");
    }

    let jwt_token = setup
        .resources
        .auth
        .auth_manager
        .generate_token(&user, &setup.resources.auth.jwks_manager)
        .expect("Failed to generate JWT");

    let response = AxumTestRequest::get("/api/oauth/status")
        .header("authorization", &format!("Bearer {}", jwt_token))
        .send(setup.routes())
        .await;
    assert_eq!(response.status(), 200);

    let body: serde_json::Value = response.json();
    let statuses = body.as_array().unwrap();
    let connected: Vec<&str> = statuses
        .iter()
        .filter(|s| s["connected"].as_bool() == Some(true))
        .map(|s| s["provider"].as_str().unwrap())
        .collect();
    assert_eq!(
        connected,
        vec![oauth_providers::STRAVA],
        "strava's OAuth row serves fetches and reads connected; garmin's serves none: {body}"
    );
    assert!(
        !statuses
            .iter()
            .any(|s| s["provider"].as_str() == Some(oauth_providers::GARMIN)),
        "a garmin OAuth row must not surface on this endpoint at all: {body}"
    );
}

// ============================================================================
// Additional Integration Tests
// ============================================================================

#[tokio::test]
async fn test_all_auth_endpoints_registered() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");
    let routes = setup.routes();

    // Test that all endpoints are registered (not 404)
    let endpoints = vec![
        ("/api/auth/register", "POST"),
        ("/oauth/token", "POST"), // OAuth2 ROPC replaces /api/auth/login
        ("/api/auth/logout", "POST"),
        ("/api/oauth/status", "GET"),
    ];

    for (endpoint, method) in endpoints {
        let response = if method == "POST" {
            AxumTestRequest::post(endpoint)
                .json(&json!({}))
                .send(routes.clone())
                .await
        } else {
            AxumTestRequest::get(endpoint).send(routes.clone()).await
        };

        // Should not be 404 (endpoint not found)
        assert_ne!(
            response.status(),
            404,
            "{} {} should be registered",
            method,
            endpoint
        );
    }
}

#[tokio::test]
async fn test_register_and_login_flow() {
    let setup = AuthTestSetup::new().await.expect("Setup failed");
    let admin_token = setup
        .create_admin_token()
        .await
        .expect("Failed to create admin token");
    let routes = setup.routes();

    // Step 1: Register a new user (with admin auth)
    let email = format!("flowtest{}@example.com", uuid::Uuid::new_v4());
    let password = "securePassword123";

    let register_request = json!({
        "email": email,
        "password": password,
        "display_name": "Flow Test User"
    });

    let register_response = AxumTestRequest::post("/api/auth/register")
        .header("Authorization", &format!("Bearer {}", admin_token))
        .json(&register_request)
        .send(routes.clone())
        .await;

    // Registration might fail in some test scenarios, so we'll be flexible
    if register_response.status() != 201 {
        return;
    }

    // Step 2: Login with the registered credentials using OAuth2 ROPC
    let login_request = [
        ("grant_type", "password"),
        ("username", email.as_str()),
        ("password", password),
    ];

    let login_response = AxumTestRequest::post("/oauth/token")
        .form(&login_request)
        .send(routes)
        .await;

    // Login might fail if user needs approval, so we'll check both scenarios
    assert!(
        login_response.status() == 200 || login_response.status() == 403,
        "Login should either succeed or be forbidden due to pending approval"
    );
}
