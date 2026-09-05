// ABOUTME: E2e coverage for the per-email pre-approval allow-list (pierre-cli user allow)
// ABOUTME: Register lands Active with operator attribution; disallow and pending-login paths covered
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The per-email allow-list closes half of registre#43: an operator records an
//! allow for an address *before* the person has an account (`pierre-cli user
//! allow --email X`), and the registration approval decision consumes it —
//! the account lands `Active` with `approved_by` attributed to the operator,
//! skipping the pending queue.
//!
//! These tests drive the real `/api/auth/register` handler and the ROPC
//! `/oauth/token` login over one in-memory database, with global
//! auto-approval off and no `AUTO_APPROVE_DOMAINS` — so any `Active` outcome
//! here is the allow-list's doing, and a returns-empty stub of the repository
//! would fail the attribution and status assertions.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use helpers::axum_test::AxumTestRequest;
use pierre_config::environment::{
    AppBehaviorConfig, BackupConfig, DatabaseConfig, DatabaseUrl, Environment, SecurityConfig,
    SecurityHeadersConfig, ServerConfig,
};
use pierre_mcp_server::mcp::resources::{ServerContext, ServerContextOptions};
use pierre_routes_auth::AuthRoutes;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

/// Build a real `ServerContext` over an in-memory `SQLite` DB with public
/// self-registration left in the pending → admin-approval mode (mirrors
/// `register_approval_login_e2e_test::setup`).
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

/// Register a real account to act as the allow-listing operator:
/// `users.approved_by` references `users.id`, so attribution needs a row.
async fn register_operator(resources: &Arc<ServerContext>, email: &str) -> Uuid {
    let (status, body) = register(resources, email, "operatorPassword123").await;
    assert_eq!(status, 201, "operator registration must succeed: {body}");
    Uuid::parse_str(body["user_id"].as_str().expect("user_id")).expect("user_id must be a UUID")
}

/// POST the real registration handler and return `(status, body)`.
async fn register(resources: &Arc<ServerContext>, email: &str, password: &str) -> (u16, Value) {
    let auth_routes = AuthRoutes::routes(resources.auth_routes_context());
    let resp = AxumTestRequest::post("/api/auth/register")
        .json(&json!({
            "email": email,
            "password": password,
            "display_name": "Allowlist Tester"
        }))
        .send(auth_routes)
        .await;
    let status = resp.status();
    (status, resp.json())
}

#[tokio::test]
async fn preapproved_email_registers_active_with_attribution() {
    let resources = setup().await.expect("server context setup failed");
    let operator = register_operator(&resources, "allow-operator-1@example.com").await;
    let email = "alpha-cohort-1@example.com";

    let added = resources
        .common
        .repos
        .pre_approved_emails
        .allow(email, Some(operator), Some("alpha cohort"))
        .await
        .expect("recording the allow must succeed");
    assert!(added, "first allow of a new address must report insertion");

    let (status, body) = register(&resources, email, "securePassword123").await;
    assert_eq!(status, 201, "registration must succeed: {body}");
    assert_eq!(
        body["user_status"].as_str(),
        Some("active"),
        "a pre-approved email must land active, got: {body}"
    );

    let user = resources
        .common
        .repos
        .users
        .get_by_email(email)
        .await
        .expect("user lookup must succeed")
        .expect("registered user must exist");
    assert_eq!(
        user.approved_by,
        Some(operator),
        "approved_by must attribute the allow-listing operator"
    );
    assert!(
        user.approved_at.is_some(),
        "approved_at must be stamped on auto-approval"
    );
}

#[tokio::test]
async fn unlisted_email_registers_pending() {
    let resources = setup().await.expect("server context setup failed");

    let (status, body) = register(&resources, "stranger@example.com", "securePassword123").await;
    assert_eq!(status, 201, "registration must succeed: {body}");
    assert_eq!(
        body["user_status"].as_str(),
        Some("pending"),
        "an unlisted email must land in the pending queue, got: {body}"
    );
}

#[tokio::test]
async fn allow_is_case_insensitive() {
    let resources = setup().await.expect("server context setup failed");
    let repos = &resources.common.repos;

    repos
        .pre_approved_emails
        .allow("Cohort.Runner@Example.COM", None, None)
        .await
        .expect("recording the allow must succeed");

    let entry = repos
        .pre_approved_emails
        .get("cohort.runner@example.com")
        .await
        .expect("lookup must succeed")
        .expect("lowercase lookup must find the mixed-case allow");
    assert_eq!(
        entry.email, "cohort.runner@example.com",
        "the stored key must be lowercase"
    );

    let (status, body) =
        register(&resources, "cohort.runner@example.com", "securePassword123").await;
    assert_eq!(status, 201, "registration must succeed: {body}");
    assert_eq!(
        body["user_status"].as_str(),
        Some("active"),
        "case must not defeat the allow-list, got: {body}"
    );
}

#[tokio::test]
async fn disallow_removes_standing_entry() {
    let resources = setup().await.expect("server context setup failed");
    let repos = &resources.common.repos;
    let email = "revoked@example.com";

    repos
        .pre_approved_emails
        .allow(email, None, None)
        .await
        .expect("recording the allow must succeed");
    assert!(
        repos
            .pre_approved_emails
            .remove(email)
            .await
            .expect("removal must succeed"),
        "removing a standing allow must report deletion"
    );
    assert!(
        !repos
            .pre_approved_emails
            .remove(email)
            .await
            .expect("second removal must succeed"),
        "removing an absent allow must report nothing deleted"
    );

    let (status, body) = register(&resources, email, "securePassword123").await;
    assert_eq!(status, 201, "registration must succeed: {body}");
    assert_eq!(
        body["user_status"].as_str(),
        Some("pending"),
        "a revoked allow must leave registration pending, got: {body}"
    );
}

#[tokio::test]
async fn allow_is_idempotent_and_listable() {
    let resources = setup().await.expect("server context setup failed");
    let repos = &resources.common.repos;
    let operator = Uuid::new_v4();

    assert!(repos
        .pre_approved_emails
        .allow("dupe@example.com", Some(operator), Some("first"))
        .await
        .expect("first allow must succeed"));
    assert!(
        !repos
            .pre_approved_emails
            .allow("dupe@example.com", Some(operator), Some("second"))
            .await
            .expect("second allow must succeed"),
        "re-allowing must be a no-op, not an error or a second row"
    );

    let entries = repos
        .pre_approved_emails
        .list()
        .await
        .expect("list must succeed");
    assert_eq!(entries.len(), 1, "idempotent allow must keep one row");
    assert_eq!(entries[0].email, "dupe@example.com");
    assert_eq!(
        entries[0].note.as_deref(),
        Some("first"),
        "the original allow's note must survive a duplicate allow"
    );
    assert_eq!(entries[0].allowed_by, Some(operator));
}

#[tokio::test]
async fn pending_user_promoted_on_next_login_with_attribution() {
    let resources = setup().await.expect("server context setup failed");
    let repos = &resources.common.repos;
    let operator = register_operator(&resources, "late-allow-operator@example.com").await;
    let email = "late-allow@example.com";
    let password = "securePassword123";

    // Registered before any allow existed → pending queue.
    let (status, body) = register(&resources, email, password).await;
    assert_eq!(status, 201, "registration must succeed: {body}");
    assert_eq!(body["user_status"].as_str(), Some("pending"));

    // The operator allows the exact address afterwards.
    repos
        .pre_approved_emails
        .allow(email, Some(operator), None)
        .await
        .expect("recording the allow must succeed");

    // Next login retroactively promotes the pending account.
    let auth_routes = AuthRoutes::routes(resources.auth_routes_context());
    let login_resp = AxumTestRequest::post("/oauth/token")
        .form(&[
            ("grant_type", "password"),
            ("username", email),
            ("password", password),
        ])
        .send(auth_routes)
        .await;
    assert_eq!(
        login_resp.status(),
        200,
        "login after a late allow must succeed"
    );
    let login_body: Value = login_resp.json();
    assert_eq!(
        login_body["user"]["user_status"].as_str(),
        Some("active"),
        "login must retroactively promote the allow-listed account, got: {login_body}"
    );

    let user = repos
        .users
        .get_by_email(email)
        .await
        .expect("user lookup must succeed")
        .expect("user must exist");
    assert_eq!(
        user.approved_by,
        Some(operator),
        "retroactive promotion must attribute the allow-listing operator"
    );
}
