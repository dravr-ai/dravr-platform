// ABOUTME: HTTP coverage for the admin-token pre-approval routes behind pierre-cli user allow
// ABOUTME: Allow/list/remove, pending promotion, operator attribution, the ManageUsers gate, bad input
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `/admin/pre-approved-emails` — the transport registre#110 was missing.
//!
//! The `pre_approved_emails` table and its repository shipped with
//! `pierre-cli user allow`, but the verb held a `DATABASE_URL` handle, so it
//! could only ever write the laptop's local database: a deployed environment's
//! Cloud SQL is on a private IP, and an operator with a valid super-admin
//! device login had no way to pre-approve an address there. These tests drive
//! the real bearer-token routes the CLI now calls, over one in-memory
//! database.
//!
//! Every assertion is on content — the recorded note, the resolved operator,
//! the promoted account's status and `approved_by`, the emptied list — so a
//! handler that answered 200 with nothing written would fail them.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use anyhow::Result;
use helpers::axum_test::AxumTestRequest;
use pierre_auth::admin::jwks::JwksManager;
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_core::admin::models::{AdminPermission, CreateAdminTokenRequest};
use pierre_core::models::{User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_routes_admin::auth::service::AdminAuthService;
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit, AdminRoutes};
use pierre_tool_runtime::guardian::GuardianConfigRegistry;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

const TEST_ADMIN_JWT_SECRET: &str = "test_jwt_secret_for_pre_approved_email_routes";

/// One in-memory database, its admin router, and the operator identity whose
/// device-login token the CLI would be carrying.
struct Harness {
    router: axum::Router,
    repos: Arc<RepositoryRegistry>,
    operator_id: Uuid,
    operator_email: String,
    jwks: Arc<JwksManager>,
}

impl Harness {
    async fn new() -> Result<Self> {
        let database = common::create_test_database().await?;
        let auth_manager = common::create_test_auth_manager();
        let jwks = common::get_shared_test_jwks();

        let database_arc = Arc::new((*database).clone());
        let repos = Arc::new(database_arc.repositories());

        let context = AdminApiContext::new(AdminApiContextInit {
            database: Arc::clone(&database_arc),
            repos: Arc::clone(&repos),
            jwt_secret: TEST_ADMIN_JWT_SECRET.to_owned(),
            auth_manager: auth_manager.clone(),
            jwks_manager: Arc::clone(&jwks),
            admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
            admin_token_cache_ttl_secs: AdminAuthService::DEFAULT_CACHE_TTL_SECS,
            harness_config_registry: Arc::new(HarnessConfigRegistry::bootstrap()),
            guardian_config_registry: Arc::new(GuardianConfigRegistry::bootstrap()),
            prompt_registry: Arc::new(pierre_contremaitre::PromptRegistry::new()),
            tool_description_registry: Arc::new(pierre_contremaitre::ToolDescriptionRegistry::new()),
            evidence_registry: Arc::new(pierre_contremaitre::EvidenceRegistry::new()),
            messaging_strings_registry: Arc::new(
                pierre_contremaitre::MessagingStringsRegistry::new(),
            ),
            cageux_config_registry: Arc::new(CageuxConfigRegistry::from_env()),
            persona_contract_registry: Arc::new(PersonaContractRegistry::new()),
            training_catalogue_registry: Arc::new(
                pierre_contremaitre::TrainingCatalogueRegistry::new(),
            ),
            contremaitre_config: None,
        });

        // The super-admin who approved the device login. `pierre-cli auth
        // login` mints its token against exactly this account.
        let operator_email = "device-operator@example.com".to_owned();
        let operator_id =
            seed_user(&repos, &operator_email, UserStatus::Active, UserRole::Admin).await?;

        Ok(Self {
            router: AdminRoutes::routes(context),
            repos,
            operator_id,
            operator_email,
            jwks,
        })
    }

    /// Mint the token the device flow would have handed the CLI: super-admin,
    /// named `device-cli:<approver email>`.
    async fn device_login_token(&self) -> Result<String> {
        self.mint_token(&format!("device-cli:{}", self.operator_email), None, true)
            .await
    }

    /// Mint an admin token with an explicit permission set.
    async fn mint_token(
        &self,
        service_name: &str,
        permissions: Option<Vec<AdminPermission>>,
        is_super_admin: bool,
    ) -> Result<String> {
        let request = CreateAdminTokenRequest {
            service_name: service_name.to_owned(),
            service_description: Some("seeded by admin_pre_approved_emails_test".to_owned()),
            permissions,
            expires_in_days: Some(30),
            is_super_admin,
            tenant_id: None,
        };
        let generated = self
            .repos
            .admin
            .create_token(&request, TEST_ADMIN_JWT_SECRET, &self.jwks)
            .await?;
        Ok(generated.jwt_token)
    }

    async fn allow(&self, token: &str, email: &str, note: Option<&str>) -> (u16, Value) {
        let response = AxumTestRequest::post("/admin/pre-approved-emails")
            .header("Authorization", &format!("Bearer {token}"))
            .json(&json!({ "email": email, "note": note }))
            .send(self.router.clone())
            .await;
        let status = response.status();
        (status, response.json())
    }

    async fn list(&self, token: &str) -> (u16, Value) {
        let response = AxumTestRequest::get("/admin/pre-approved-emails")
            .header("Authorization", &format!("Bearer {token}"))
            .send(self.router.clone())
            .await;
        let status = response.status();
        (status, response.json())
    }

    async fn disallow(&self, token: &str, email: &str) -> (u16, Value) {
        let path = format!("/admin/pre-approved-emails/{}", urlencoding::encode(email));
        let response = AxumTestRequest::delete(&path)
            .header("Authorization", &format!("Bearer {token}"))
            .send(self.router.clone())
            .await;
        let status = response.status();
        (status, response.json())
    }
}

/// Create a user row directly; registration itself is covered by
/// `preapproved_email_registration_test`.
async fn seed_user(
    repos: &Arc<RepositoryRegistry>,
    email: &str,
    status: UserStatus,
    role: UserRole,
) -> Result<Uuid> {
    let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST)?;
    let mut user = User::new(email.to_owned(), password_hash, Some("Seeded".to_owned()));
    user.user_status = status;
    user.role = role;
    user.is_admin = role.is_admin_or_higher();
    let id = user.id;
    repos.users.create(&user).await?;
    Ok(id)
}

fn entries(body: &Value) -> Vec<Value> {
    body["data"]["emails"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

#[tokio::test]
async fn allow_list_and_remove_round_trip_over_http() -> Result<()> {
    let harness = Harness::new().await?;
    let token = harness.device_login_token().await?;
    let email = "alpha-cohort@example.com";

    let (status, body) = harness.allow(&token, email, Some("alpha cohort")).await;
    assert_eq!(status, 200, "allow must succeed: {body}");
    assert_eq!(
        body["data"]["outcome"].as_str(),
        Some("recorded"),
        "an address with no account records a standing allow: {body}"
    );

    let (status, body) = harness.list(&token).await;
    assert_eq!(status, 200, "listing must succeed: {body}");
    let listed = entries(&body);
    assert_eq!(
        listed.len(),
        1,
        "exactly the one allow must be listed: {body}"
    );
    assert_eq!(listed[0]["email"].as_str(), Some(email));
    assert_eq!(
        listed[0]["note"].as_str(),
        Some("alpha cohort"),
        "the operator note must survive the round trip"
    );
    assert!(
        listed[0]["account_status"].is_null(),
        "nobody has registered against the address yet: {body}"
    );
    assert_eq!(
        listed[0]["allowed_by_email"].as_str(),
        Some(harness.operator_email.as_str()),
        "the device-login token must attribute the allow to its approving operator"
    );
    assert_eq!(
        listed[0]["allowed_by"].as_str(),
        Some(harness.operator_id.to_string().as_str()),
        "attribution must carry the operator's real user id"
    );

    let (status, body) = harness.disallow(&token, email).await;
    assert_eq!(status, 200, "removal must succeed: {body}");
    assert_eq!(
        body["data"]["removed"].as_bool(),
        Some(true),
        "removing a standing allow must report the deletion: {body}"
    );

    let (_, body) = harness.list(&token).await;
    assert!(
        entries(&body).is_empty(),
        "the removed allow must be gone from the listing: {body}"
    );

    let (status, body) = harness.disallow(&token, email).await;
    assert_eq!(status, 200, "a second removal is not an error: {body}");
    assert_eq!(
        body["data"]["removed"].as_bool(),
        Some(false),
        "removing an absent allow must report nothing deleted: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn allow_approves_a_pending_account_and_provisions_its_token() -> Result<()> {
    let harness = Harness::new().await?;
    let token = harness.device_login_token().await?;
    let email = "waiting-in-the-queue@example.com";
    let user_id = seed_user(&harness.repos, email, UserStatus::Pending, UserRole::User).await?;

    let (status, body) = harness.allow(&token, email, None).await;
    assert_eq!(status, 200, "allow must succeed: {body}");
    assert_eq!(
        body["data"]["outcome"].as_str(),
        Some("pending_approved"),
        "an address with a pending account is approved on the spot: {body}"
    );
    assert_eq!(
        body["data"]["approved_user_id"].as_str(),
        Some(user_id.to_string().as_str()),
        "the response must name the account it promoted: {body}"
    );

    let user = harness
        .repos
        .users
        .get_by_email(email)
        .await?
        .expect("the seeded user must still exist");
    assert_eq!(
        user.user_status,
        UserStatus::Active,
        "the pending account must be active after the allow"
    );
    assert_eq!(
        user.approved_by,
        Some(harness.operator_id),
        "the promotion must be attributed to the acting operator"
    );

    let tokens = harness.repos.user_mcp_tokens.list_tokens(user_id).await?;
    assert_eq!(
        tokens.len(),
        1,
        "approval must provision the default MCP token, exactly as user approve does"
    );
    Ok(())
}

#[tokio::test]
async fn listing_reports_registration_state_per_address() -> Result<()> {
    let harness = Harness::new().await?;
    let token = harness.device_login_token().await?;
    let registered = "already-here@example.com";
    let waiting = "not-yet@example.com";

    seed_user(
        &harness.repos,
        registered,
        UserStatus::Active,
        UserRole::User,
    )
    .await?;
    // Allowing an active account is a no-op on the list, so record the row
    // directly: the listing must still report the account it found.
    harness
        .repos
        .pre_approved_emails
        .allow(registered, Some(harness.operator_id), None)
        .await?;
    let (status, body) = harness.allow(&token, waiting, None).await;
    assert_eq!(status, 200, "the second allow must succeed: {body}");

    let (_, body) = harness.list(&token).await;
    let listed = entries(&body);
    assert_eq!(listed.len(), 2, "both allows must be listed: {body}");

    let by_email = |wanted: &str| -> Value {
        listed
            .iter()
            .find(|e| e["email"].as_str() == Some(wanted))
            .cloned()
            .unwrap_or(Value::Null)
    };
    assert_eq!(
        by_email(registered)["account_status"].as_str(),
        Some("active"),
        "an allow whose person registered must report their account status: {body}"
    );
    assert!(
        by_email(waiting)["account_status"].is_null(),
        "an allow still waiting must report no account: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn allow_is_denied_without_manage_users_and_writes_nothing() -> Result<()> {
    let harness = Harness::new().await?;
    let weak = harness
        .mint_token(
            "list-keys-only-service",
            Some(vec![AdminPermission::ListKeys]),
            false,
        )
        .await?;

    let (status, body) = harness.allow(&weak, "sneaky@example.com", None).await;
    assert_eq!(
        status, 403,
        "a token without ManageUsers must not pre-approve anyone: {body}"
    );

    let (status, body) = harness.list(&weak).await;
    assert_eq!(status, 403, "the listing is gated the same way: {body}");

    let stored = harness.repos.pre_approved_emails.list().await?;
    assert!(
        stored.is_empty(),
        "the rejected allow must not have reached the table"
    );
    Ok(())
}

#[tokio::test]
async fn a_malformed_address_is_rejected_before_it_is_stored() -> Result<()> {
    let harness = Harness::new().await?;
    let token = harness.device_login_token().await?;

    let (status, body) = harness.allow(&token, "not-an-email", None).await;
    assert_eq!(
        status, 400,
        "a malformed address must be rejected, not stored: {body}"
    );

    let stored = harness.repos.pre_approved_emails.list().await?;
    assert!(
        stored.is_empty(),
        "nothing may be written for a rejected address"
    );
    Ok(())
}

#[tokio::test]
async fn a_suspended_account_is_left_alone_by_an_allow() -> Result<()> {
    let harness = Harness::new().await?;
    let token = harness.device_login_token().await?;
    let email = "suspended-on-purpose@example.com";
    seed_user(&harness.repos, email, UserStatus::Suspended, UserRole::User).await?;

    let (status, body) = harness.allow(&token, email, None).await;
    assert_eq!(status, 200, "the call itself succeeds: {body}");
    assert_eq!(
        body["data"]["outcome"].as_str(),
        Some("suspended_unchanged"),
        "suspension is reversed explicitly, never as a side effect of an allow: {body}"
    );

    let user = harness
        .repos
        .users
        .get_by_email(email)
        .await?
        .expect("the suspended user must still exist");
    assert_eq!(
        user.user_status,
        UserStatus::Suspended,
        "the suspended account must be untouched"
    );
    Ok(())
}
