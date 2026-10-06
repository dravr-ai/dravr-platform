// ABOUTME: Tests for the login algorithm's internal logic and security properties
// ABOUTME: Validates user status checks, password verification, timestamp updates, and error uniformity
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(clippy::uninlined_format_args)]

//! Login Algorithm Tests
//!
//! These tests exercise the login algorithm's internal logic through the
//! first-party sign-in (carnet#787): the password is checked on the hosted
//! login page (`POST /oauth2/login`), and a sign-in it admits ends in the
//! session `/oauth/token` mints for the app's authorization code. They focus on:
//! - User status checks (Active, Pending, Suspended)
//! - Password verification edge cases
//! - `last_active` timestamp updates
//! - Error message uniformity (preventing information leakage)

mod common;
mod helpers;

use chrono::{Timelike, Utc};
use helpers::axum_test::AxumTestResponse;
use helpers::first_party_sign_in::{SignIn, SignInOutcome};
use pierre_config::environment::{
    AppBehaviorConfig, BackupConfig, DatabaseConfig, DatabaseUrl, Environment, SecurityConfig,
    SecurityHeadersConfig, ServerConfig,
};
use pierre_core::models::{User, UserStatus};
use pierre_database::backends::factory::Database;
use pierre_mcp_server::mcp::resources::{ServerContext, ServerContextOptions};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

/// Test setup helper for login algorithm testing
struct LoginAlgorithmTestSetup {
    resources: Arc<ServerContext>,
    database: Arc<Database>,
}

impl LoginAlgorithmTestSetup {
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
            .await
            .unwrap(),
        );

        Ok(Self {
            resources,
            database,
        })
    }

    /// Sign `email` in with `password` the way Dravr's web app does: the
    /// hosted login page, then the authorization code redeemed with PKCE.
    async fn sign_in(&self, email: &str, password: &str) -> SignInOutcome {
        SignIn::new(email, password).run(&self.resources).await
    }

    /// Create a user with a specific status and password
    async fn create_user_with_status(
        &self,
        email: &str,
        password: &str,
        status: UserStatus,
    ) -> anyhow::Result<User> {
        let password_hash = bcrypt::hash(password, bcrypt::DEFAULT_COST)?;
        let mut user = User::new(
            email.to_owned(),
            password_hash,
            Some("Test User".to_owned()),
        );
        user.user_status = status;

        // Set approval fields for Active users
        if status == UserStatus::Active {
            user.approved_by = Some(user.id);
            user.approved_at = Some(Utc::now());
        }

        self.database.repositories().users.create(&user).await?;
        Ok(user)
    }
}

/// The hosted login form's generic refusal: no session, and nothing that
/// tells a wrong password from an unknown account.
const GENERIC_REFUSAL: &str = "Invalid email or password";

/// Assert the hosted login form refused the sign-in with its generic page,
/// and set no session.
fn assert_refused_generically(refused: &AxumTestResponse) {
    assert_eq!(refused.status(), 401, "{}", refused.body_text());
    assert!(
        refused.body_text().contains(GENERIC_REFUSAL),
        "{}",
        refused.body_text()
    );
    assert!(
        refused.header("set-cookie").is_none(),
        "a refused sign-in never sets a session"
    );
}

// ============================================================================
// User Status Tests - Verify login behavior for different account states
// ============================================================================

#[tokio::test]
async fn test_login_active_user_succeeds() {
    let setup = LoginAlgorithmTestSetup::new().await.expect("Setup failed");

    let email = "active@example.com";
    let password = "securePassword123";

    setup
        .create_user_with_status(email, password, UserStatus::Active)
        .await
        .expect("Failed to create user");

    let body = setup.sign_in(email, password).await.signed_in();
    assert!(body["access_token"].is_string());
    assert_eq!(body["user"]["email"].as_str(), Some(email));
}

#[tokio::test]
async fn test_login_pending_user_succeeds_with_status_in_response() {
    // DESIGN NOTE: Pending users CAN authenticate - the frontend restricts access
    // based on user_status in the response. This is intentional to allow the
    // frontend to display appropriate messaging (e.g., "Your account is pending approval")
    let setup = LoginAlgorithmTestSetup::new().await.expect("Setup failed");

    let email = "pending@example.com";
    let password = "securePassword123";

    setup
        .create_user_with_status(email, password, UserStatus::Pending)
        .await
        .expect("Failed to create user");

    // Pending users authenticate: the hosted form admits them, /oauth2/authorize
    // issues the app its code, and access control is the frontend's.
    let body = setup.sign_in(email, password).await.signed_in();

    // Verify the user_status is returned so frontend can act on it
    let user_status = body["user"]["user_status"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase();

    assert!(
        user_status.contains("pending"),
        "Response should include user_status=Pending for frontend handling, got: {}",
        user_status
    );
}

#[tokio::test]
async fn test_login_suspended_user_is_rejected() {
    // Suspended users are blocked at sign-in: the hosted login form refuses
    // them with a page that says why, and no session or code is issued.
    let setup = LoginAlgorithmTestSetup::new().await.expect("Setup failed");

    let email = "suspended@example.com";
    let password = "securePassword123";

    setup
        .create_user_with_status(email, password, UserStatus::Suspended)
        .await
        .expect("Failed to create user");

    let refused = setup.sign_in(email, password).await.login_refused();

    assert!(
        refused.status() >= 400,
        "Suspended user should be refused, got {}",
        refused.status()
    );
    assert!(
        refused.header("set-cookie").is_none(),
        "a suspended account gets no session"
    );
    let page = refused.body_text();
    assert!(
        !page.contains(GENERIC_REFUSAL),
        "A suspended account's right password is not called wrong, got: {page}"
    );
    assert!(
        page.to_lowercase().contains("suspended"),
        "The refusal should mention suspension, got: {page}"
    );
}

// ============================================================================
// Password Verification Tests - Edge cases in password checking
// ============================================================================

#[tokio::test]
async fn test_login_correct_password_succeeds() {
    let setup = LoginAlgorithmTestSetup::new().await.expect("Setup failed");

    let email = "correct_pw@example.com";
    let password = "correctPassword123";

    setup
        .create_user_with_status(email, password, UserStatus::Active)
        .await
        .expect("Failed to create user");

    let response = setup.sign_in(email, password).await.token();
    assert_eq!(response.status(), 200, "Correct password should succeed");
}

#[tokio::test]
async fn test_login_wrong_password_fails() {
    let setup = LoginAlgorithmTestSetup::new().await.expect("Setup failed");

    let email = "wrong_pw@example.com";
    let correct_password = "correctPassword123";
    let wrong_password = "wrongPassword456";

    setup
        .create_user_with_status(email, correct_password, UserStatus::Active)
        .await
        .expect("Failed to create user");

    // The hosted login form refuses a wrong password; no code reaches the app.
    let refused = setup.sign_in(email, wrong_password).await.login_refused();
    assert_refused_generically(&refused);
}

#[tokio::test]
async fn test_login_empty_password_fails() {
    let setup = LoginAlgorithmTestSetup::new().await.expect("Setup failed");

    let email = "empty_pw@example.com";
    let password = "realPassword123";

    setup
        .create_user_with_status(email, password, UserStatus::Active)
        .await
        .expect("Failed to create user");

    let refused = setup.sign_in(email, "").await.login_refused();
    assert_ne!(refused.status(), 200, "Empty password should not succeed");
    assert!(
        refused.header("set-cookie").is_none(),
        "an empty password gets no session"
    );
}

#[tokio::test]
async fn test_login_case_sensitive_password() {
    let setup = LoginAlgorithmTestSetup::new().await.expect("Setup failed");

    let email = "case_sensitive@example.com";
    let password = "CaseSensitivePassword123";
    let wrong_case_password = "casesensitivepassword123";

    setup
        .create_user_with_status(email, password, UserStatus::Active)
        .await
        .expect("Failed to create user");

    // Password verification is case-sensitive
    let refused = setup
        .sign_in(email, wrong_case_password)
        .await
        .login_refused();
    assert_refused_generically(&refused);
}

#[tokio::test]
async fn test_login_unicode_password() {
    let setup = LoginAlgorithmTestSetup::new().await.expect("Setup failed");

    let email = "unicode_pw@example.com";
    let password = "пароль密码🔐123"; // Russian, Chinese, and emoji

    setup
        .create_user_with_status(email, password, UserStatus::Active)
        .await
        .expect("Failed to create user");

    let response = setup.sign_in(email, password).await.token();
    assert_eq!(
        response.status(),
        200,
        "Unicode passwords should work correctly"
    );
}

// ============================================================================
// Last Active Timestamp Tests - Verify timestamp updates on login
// ============================================================================

#[tokio::test]
async fn test_login_updates_last_active_timestamp() {
    let setup = LoginAlgorithmTestSetup::new().await.expect("Setup failed");

    let email = "timestamp@example.com";
    let password = "securePassword123";

    let user = setup
        .create_user_with_status(email, password, UserStatus::Active)
        .await
        .expect("Failed to create user");

    // Record time before login, truncated to seconds since SQLite stores
    // timestamps with second-level precision
    let before_login = Utc::now().with_nanosecond(0).expect("valid timestamp");

    // Delay to ensure timestamp difference across second boundary
    sleep(Duration::from_millis(1100)).await;

    let response = setup.sign_in(email, password).await.token();
    assert_eq!(response.status(), 200);

    // Fetch the updated user from database
    let updated_user = setup
        .database
        .repositories()
        .users
        .get_global(user.id)
        .await
        .expect("Failed to get user")
        .expect("User should exist");

    assert!(
        updated_user.last_active > before_login,
        "last_active should be updated after login. Before: {}, After: {}",
        before_login,
        updated_user.last_active
    );
}

#[tokio::test]
async fn test_failed_login_does_not_update_timestamp() {
    let setup = LoginAlgorithmTestSetup::new().await.expect("Setup failed");

    let email = "no_timestamp@example.com";
    let password = "securePassword123";

    let user = setup
        .create_user_with_status(email, password, UserStatus::Active)
        .await
        .expect("Failed to create user");

    // Get initial last_active
    let initial_user = setup
        .database
        .repositories()
        .users
        .get_global(user.id)
        .await
        .expect("Failed to get user")
        .expect("User should exist");
    let initial_last_active = initial_user.last_active;

    // Small delay
    sleep(Duration::from_millis(10)).await;

    // Attempt login with wrong password
    let refused = setup.sign_in(email, "wrongPassword").await.login_refused();
    assert_refused_generically(&refused);

    // Verify timestamp was NOT updated
    let after_user = setup
        .database
        .repositories()
        .users
        .get_global(user.id)
        .await
        .expect("Failed to get user")
        .expect("User should exist");

    assert_eq!(
        after_user.last_active, initial_last_active,
        "last_active should NOT be updated on failed login"
    );
}

// ============================================================================
// Error Message Uniformity Tests - Prevent information leakage
// ============================================================================

#[tokio::test]
async fn test_nonexistent_user_error_matches_wrong_password_error() {
    let setup = LoginAlgorithmTestSetup::new().await.expect("Setup failed");

    let existing_email = "existing@example.com";
    let nonexistent_email = "nonexistent@example.com";
    let password = "securePassword123";

    // Create one user
    setup
        .create_user_with_status(existing_email, password, UserStatus::Active)
        .await
        .expect("Failed to create user");

    // Try login with nonexistent user
    let nonexistent = setup
        .sign_in(nonexistent_email, password)
        .await
        .login_refused();

    // Try login with existing user but wrong password
    let wrong_pw = setup
        .sign_in(existing_email, "wrongPassword")
        .await
        .login_refused();

    // Both should return the same status code
    assert_eq!(
        nonexistent.status(),
        wrong_pw.status(),
        "Nonexistent user and wrong password should return same status code"
    );

    // Both pages carry the same generic message, so neither tells the two apart
    assert_refused_generically(&nonexistent);
    assert_refused_generically(&wrong_pw);

    // The pages differ only in the authorization request they echo back
    // (state and PKCE challenge are fresh per sign-in), never in the message.
    let message = |page: &str| {
        page.lines()
            .filter(|line| line.contains(GENERIC_REFUSAL))
            .map(str::trim)
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(
        message(&nonexistent.body_text()),
        message(&wrong_pw.body_text()),
        "Error messages should match to prevent user enumeration"
    );
}

#[tokio::test]
async fn test_error_does_not_reveal_user_exists() {
    let setup = LoginAlgorithmTestSetup::new().await.expect("Setup failed");

    let email = "secret_exists@example.com";
    let password = "securePassword123";

    setup
        .create_user_with_status(email, password, UserStatus::Active)
        .await
        .expect("Failed to create user");

    // Try login with wrong password
    let refused = setup.sign_in(email, "wrongPassword").await.login_refused();
    let page = refused.body_text().to_lowercase();

    // Should NOT contain phrases that reveal the user exists
    assert!(
        !page.contains("user exists"),
        "Error should not reveal user exists"
    );
    assert!(
        !page.contains("password incorrect"),
        "Error should not specifically mention password is wrong"
    );
    assert!(
        !page.contains("wrong password"),
        "Error should not specifically mention wrong password"
    );
}

// ============================================================================
// Additional Edge Cases
// ============================================================================

#[tokio::test]
async fn test_login_with_whitespace_in_email() {
    let setup = LoginAlgorithmTestSetup::new().await.expect("Setup failed");

    let email = "whitespace@example.com";
    let password = "securePassword123";

    setup
        .create_user_with_status(email, password, UserStatus::Active)
        .await
        .expect("Failed to create user");

    // Try login with leading/trailing whitespace in email
    let body = setup
        .sign_in(" whitespace@example.com ", password)
        .await
        .signed_in();

    // An email is compared in its normalized form, so surrounding
    // whitespace does not stop the account's own address from signing in.
    assert_eq!(body["user"]["email"], "whitespace@example.com", "{body}");
}

#[tokio::test]
async fn test_login_case_insensitive_email() {
    let setup = LoginAlgorithmTestSetup::new().await.expect("Setup failed");

    let email = "CaseMixed@Example.COM";
    let password = "securePassword123";

    let user = setup
        .create_user_with_status(email, password, UserStatus::Active)
        .await
        .expect("Failed to create user");

    // Emails are case-insensitive: the account registered as
    // `CaseMixed@Example.COM` is stored lowercase and signs in in any casing.
    let body = setup
        .sign_in("casemixed@example.com", password)
        .await
        .signed_in();
    assert_eq!(body["user"]["user_id"], user.id.to_string(), "{body}");
    assert_eq!(body["user"]["email"], "casemixed@example.com", "{body}");
}

#[tokio::test]
async fn test_multiple_failed_logins_same_user() {
    let setup = LoginAlgorithmTestSetup::new().await.expect("Setup failed");

    let email = "multiple_fails@example.com";
    let password = "securePassword123";

    setup
        .create_user_with_status(email, password, UserStatus::Active)
        .await
        .expect("Failed to create user");

    // Attempt multiple failed logins
    for i in 0..5 {
        let refused = setup.sign_in(email, "wrongPassword").await.login_refused();
        assert_eq!(refused.status(), 401, "Attempt {} should fail", i + 1);
    }

    // Five refusals stay inside the account's window of refused passwords
    // (PASSWORD_LOGIN_ACCOUNT_RPM = 10), so the right password still signs in.
    let response = setup.sign_in(email, password).await.token();
    assert_eq!(
        response.status(),
        200,
        "Correct password should succeed after failures inside the window: {}",
        response.body_text()
    );
}

#[tokio::test]
async fn test_login_response_contains_required_fields() {
    let setup = LoginAlgorithmTestSetup::new().await.expect("Setup failed");

    let email = "fields_check@example.com";
    let password = "securePassword123";

    setup
        .create_user_with_status(email, password, UserStatus::Active)
        .await
        .expect("Failed to create user");

    let body = setup.sign_in(email, password).await.signed_in();

    // OAuth2 required fields
    assert!(
        body["access_token"].is_string(),
        "Response must contain access_token"
    );
    assert!(
        body["token_type"].is_string(),
        "Response must contain token_type"
    );
    assert!(
        body["expires_in"].is_number(),
        "Response must contain expires_in"
    );

    // User info fields
    assert!(
        body["user"]["user_id"].is_string(),
        "Response must contain user.user_id"
    );
    assert!(
        body["user"]["email"].is_string(),
        "Response must contain user.email"
    );
    assert!(
        body["user"]["user_status"].is_string(),
        "Response must contain user.user_status"
    );
}
