// ABOUTME: Self-serve sign-up guards end to end: kill switch, 24-hour ceiling, per-address window, Turnstile, verified activation
// ABOUTME: Drives the real register, signup-config, verify-email and admin settings routes over one database (carnet#1063)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Self-serve sign-up stays bounded once it can activate itself:
//!
//! 1. the operator's kill switch closes every self-serve door, and the
//!    operator's own door (the admin register route) stays open;
//! 2. the 24-hour ceiling refuses past the accounts it admits;
//!    both refusals are a 429 reading the paused message;
//! 3. one client address gets a window of sign-ups, and another address its own;
//! 4. with Turnstile configured, a missing or refused token is refused and
//!    an outage at Cloudflare is not;
//! 5. with sign-up open to verified addresses, proving the address — by the
//!    link, or before a later sign-in — is what activates the account, and an
//!    unproven one never activates;
//! 6. the admin settings route reads and writes all three knobs.

use crate::common;
use crate::helpers;
use secrecy::{ExposeSecret, SecretString};

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::StatusCode;
use axum::Router;
use helpers::axum_test::AxumTestRequest;
use pierre_auth::dto::auth::{LoginRequest, RegisterRequest};
use pierre_auth::human_check::{HumanCheck, HumanCheckVerdict};
use pierre_config::constants::oauth_rate_limiting::SIGN_UP_RPM;
use pierre_config::environment::{
    AppBehaviorConfig, BackupConfig, DatabaseConfig, DatabaseUrl, Environment, SecurityConfig,
    SecurityHeadersConfig, ServerConfig,
};
use pierre_core::admin::models::{AdminPermission, CreateAdminTokenRequest};
use pierre_core::models::UserStatus;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::{ServerContext, ServerContextOptions};
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit, AdminRoutes};
use pierre_routes_auth::AuthRoutes;
use pierre_services::link_token::generate_link_token;
use pierre_services::signup_policy::{set_signup_policy, SignupPolicyUpdate, SIGNUP_PAUSED};
use serde_json::{json, Value};
use uuid::Uuid;

const PASSWORD: &str = "securePassword123";

/// Cloudflare's published always-pass site key, as a configured widget would carry.
const TEST_SITE_KEY: &str = "1x00000000000000000000AA";

/// A real server context over an in-memory database, with registration left
/// in the pending → admin-approval posture.
async fn context() -> ServerContext {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let auth_manager = common::create_test_auth_manager();
    let cache = common::create_test_cache().await.unwrap();
    let temp_dir = tempfile::tempdir().unwrap();
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
    .unwrap()
}

/// The auth routes as served behind a TCP peer at `client`, outside every
/// trusted network, so the peer is the client the window is keyed on.
fn auth_routes_from(resources: &ServerContext, client: Ipv4Addr) -> Router {
    AuthRoutes::routes(resources.auth_routes_context())
        .layer(MockConnectInfo(SocketAddr::new(IpAddr::V4(client), 40_000)))
}

fn auth_routes(resources: &ServerContext) -> Router {
    auth_routes_from(resources, Ipv4Addr::new(203, 0, 113, 7))
}

async fn register(router: Router, email: &str, token: Option<&str>) -> (u16, Value) {
    let mut body = json!({ "email": email, "password": PASSWORD });
    if let Some(token) = token {
        body["turnstile_token"] = json!(token);
    }
    let response = AxumTestRequest::post("/api/auth/register")
        .json(&body)
        .send(router)
        .await;
    let status = response.status();
    let text = response.body_text();
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

async fn signup_config(router: Router) -> Value {
    let response = AxumTestRequest::get("/api/auth/signup-config")
        .send(router)
        .await;
    assert_eq!(response.status(), 200);
    response.json()
}

async fn status_of(resources: &ServerContext, email: &str) -> UserStatus {
    resources
        .common
        .repos
        .users
        .get_by_email(email)
        .await
        .unwrap()
        .expect("the account exists")
        .user_status
}

async fn set_policy(resources: &ServerContext, update: SignupPolicyUpdate) {
    set_signup_policy(&resources.agent.database, update)
        .await
        .unwrap();
}

/// A bot check whose every token gets `verdict`.
struct FakeCheck {
    verdict: HumanCheckVerdict,
}

#[async_trait]
impl HumanCheck for FakeCheck {
    fn site_key(&self) -> &str {
        TEST_SITE_KEY
    }

    async fn verify(&self, _token: &str, _remote_ip: Option<IpAddr>) -> HumanCheckVerdict {
        self.verdict
    }
}

async fn context_with_check(verdict: HumanCheckVerdict) -> ServerContext {
    let mut resources = context().await;
    resources.auth.human_check = Some(Arc::new(FakeCheck { verdict }));
    resources
}

// ---------------------------------------------------------------------------
// 1. The kill switch
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_closed_switch_refuses_the_register_form_and_says_so_up_front() {
    let resources = context().await;
    set_policy(
        &resources,
        SignupPolicyUpdate {
            open: Some(false),
            ..Default::default()
        },
    )
    .await;

    let (status, body) = register(auth_routes(&resources), "closed@example.com", None).await;
    assert_eq!(
        status, 429,
        "a paused sign-up is a spent quota, told in words"
    );
    assert!(
        body.to_string().contains(SIGNUP_PAUSED),
        "the refusal reads the paused message: {body}"
    );
    assert!(
        resources
            .common
            .repos
            .users
            .get_by_email("closed@example.com")
            .await
            .unwrap()
            .is_none(),
        "a refused sign-up writes no account"
    );
    assert_eq!(signup_config(auth_routes(&resources)).await["open"], false);
}

#[tokio::test]
async fn a_closed_switch_closes_the_stranger_doors_but_not_the_operator_door() {
    let resources = context().await;
    set_policy(
        &resources,
        SignupPolicyUpdate {
            open: Some(false),
            ..Default::default()
        },
    )
    .await;
    let service = resources.account_service();
    let request = |email: &str| RegisterRequest {
        email: email.to_owned(),
        password: SecretString::from(PASSWORD.to_owned()),
        display_name: None,
    };

    let refused = service
        .self_register(request("stranger@example.com"))
        .await
        .expect_err("the link page and in-chat sign-up go through self_register");
    assert!(refused.message.ends_with(SIGNUP_PAUSED));

    service
        .register(request("operator-made@example.com"))
        .await
        .expect("an operator's register is not a self-serve sign-up");
}

#[tokio::test]
async fn reopening_the_switch_admits_sign_ups_again() {
    let resources = context().await;
    set_policy(
        &resources,
        SignupPolicyUpdate {
            open: Some(false),
            ..Default::default()
        },
    )
    .await;
    set_policy(
        &resources,
        SignupPolicyUpdate {
            open: Some(true),
            ..Default::default()
        },
    )
    .await;

    let (status, _) = register(auth_routes(&resources), "reopened@example.com", None).await;
    assert_eq!(status, 201);
    assert_eq!(signup_config(auth_routes(&resources)).await["open"], true);
}

// ---------------------------------------------------------------------------
// 2. The 24-hour ceiling
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_ceiling_admits_up_to_its_limit_then_refuses() {
    let resources = context().await;
    let existing = resources
        .common
        .repos
        .users
        .count_created_since(chrono::Utc::now() - chrono::Duration::hours(24))
        .await
        .unwrap();
    // Room for exactly one more account in the window.
    set_policy(
        &resources,
        SignupPolicyUpdate {
            daily_limit: Some(existing + 1),
            ..Default::default()
        },
    )
    .await;

    let (first, _) = register(
        auth_routes_from(&resources, Ipv4Addr::new(203, 0, 113, 10)),
        "under-ceiling@example.com",
        None,
    )
    .await;
    assert_eq!(first, 201, "the ceiling admits up to its limit");

    let (second, body) = register(
        auth_routes_from(&resources, Ipv4Addr::new(203, 0, 113, 11)),
        "over-ceiling@example.com",
        None,
    )
    .await;
    assert_eq!(second, 429, "past the ceiling, from any address: {body}");
    assert!(body.to_string().contains(SIGNUP_PAUSED), "{body}");
    assert_eq!(signup_config(auth_routes(&resources)).await["open"], false);
}

#[tokio::test]
async fn the_ceiling_counts_only_the_last_24_hours() {
    let resources = context().await;
    let users = &resources.common.repos.users;
    let before = users
        .count_created_since(chrono::Utc::now() - chrono::Duration::hours(24))
        .await
        .unwrap();
    let (status, _) = register(auth_routes(&resources), "counted@example.com", None).await;
    assert_eq!(status, 201);

    let window = users
        .count_created_since(chrono::Utc::now() - chrono::Duration::hours(24))
        .await
        .unwrap();
    assert_eq!(window, before + 1, "a new account falls in the window");
    let future = users
        .count_created_since(chrono::Utc::now() + chrono::Duration::hours(1))
        .await
        .unwrap();
    assert_eq!(future, 0, "nothing was created after now");
}

// ---------------------------------------------------------------------------
// 3. The per-address window
// ---------------------------------------------------------------------------

#[tokio::test]
async fn one_address_gets_a_window_of_sign_ups_and_another_address_its_own() {
    let resources = context().await;
    let crowded = Ipv4Addr::new(198, 51, 100, 20);

    for n in 0..SIGN_UP_RPM {
        let (status, body) = register(
            auth_routes_from(&resources, crowded),
            &format!("window-{n}@example.com"),
            None,
        )
        .await;
        assert_eq!(status, 201, "sign-up {n} is within the window: {body}");
    }

    let response = AxumTestRequest::post("/api/auth/register")
        .json(&json!({ "email": "window-over@example.com", "password": PASSWORD }))
        .send(auth_routes_from(&resources, crowded))
        .await;
    assert_eq!(response.status(), 429, "past the address's window");
    assert!(
        response.header("retry-after").is_some(),
        "the refusal says when to retry"
    );

    let (elsewhere, _) = register(
        auth_routes_from(&resources, Ipv4Addr::new(198, 51, 100, 21)),
        "window-elsewhere@example.com",
        None,
    )
    .await;
    assert_eq!(elsewhere, 201, "another address keeps its own window");
}

#[tokio::test]
async fn verification_resends_count_in_the_same_window() {
    let resources = context().await;
    let client = Ipv4Addr::new(198, 51, 100, 30);
    for _ in 0..SIGN_UP_RPM {
        let response = AxumTestRequest::post("/api/auth/resend-verification")
            .json(&json!({ "email": "nobody@example.com" }))
            .send(auth_routes_from(&resources, client))
            .await;
        assert_eq!(
            response.status(),
            200,
            "the neutral answer, within the window"
        );
    }
    let response = AxumTestRequest::post("/api/auth/resend-verification")
        .json(&json!({ "email": "nobody@example.com" }))
        .send(auth_routes_from(&resources, client))
        .await;
    assert_eq!(
        response.status(),
        429,
        "one address cannot spread resends across many accounts"
    );
}

// ---------------------------------------------------------------------------
// 4. The bot check
// ---------------------------------------------------------------------------

#[tokio::test]
async fn without_turnstile_keys_the_form_runs_unchecked() {
    let resources = context().await;
    let config = signup_config(auth_routes(&resources)).await;
    assert_eq!(config["turnstile_site_key"], Value::Null);
    let (status, _) = register(auth_routes(&resources), "unchecked@example.com", None).await;
    assert_eq!(status, 201);
}

#[tokio::test]
async fn a_configured_check_hands_the_form_its_site_key() {
    let resources = context_with_check(HumanCheckVerdict::Passed).await;
    assert_eq!(
        signup_config(auth_routes(&resources)).await["turnstile_site_key"],
        TEST_SITE_KEY
    );
}

#[tokio::test]
async fn a_sign_up_without_a_token_is_refused_when_the_check_is_on() {
    let resources = context_with_check(HumanCheckVerdict::Passed).await;
    let (status, _) = register(auth_routes(&resources), "no-token@example.com", None).await;
    assert_eq!(status, 403);
    let (empty, _) = register(auth_routes(&resources), "empty-token@example.com", Some("")).await;
    assert_eq!(empty, 403, "an empty token is no token");
}

#[tokio::test]
async fn a_token_cloudflare_refuses_is_refused_and_writes_no_account() {
    let resources = context_with_check(HumanCheckVerdict::Failed).await;
    let (status, _) = register(auth_routes(&resources), "bot@example.com", Some("forged")).await;
    assert_eq!(status, 403);
    assert!(resources
        .common
        .repos
        .users
        .get_by_email("bot@example.com")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn a_vouched_token_signs_up() {
    let resources = context_with_check(HumanCheckVerdict::Passed).await;
    let (status, _) = register(auth_routes(&resources), "human@example.com", Some("ok")).await;
    assert_eq!(status, 201);
}

#[tokio::test]
async fn cloudflare_out_of_reach_does_not_close_sign_up() {
    let resources = context_with_check(HumanCheckVerdict::Unavailable).await;
    let (status, _) = register(auth_routes(&resources), "outage@example.com", Some("ok")).await;
    assert_eq!(
        status, 201,
        "the window and the ceiling still bound sign-up during an outage"
    );
}

// ---------------------------------------------------------------------------
// 5. Activation on a verified address
// ---------------------------------------------------------------------------

async fn self_activating() -> ServerContext {
    let resources = context().await;
    set_policy(
        &resources,
        SignupPolicyUpdate {
            activate_on_verified_email: Some(true),
            ..Default::default()
        },
    )
    .await;
    resources
}

/// Follow a confirmation link minted for `user_id`, as the mailed one is.
async fn follow_confirmation_link(resources: &ServerContext, user_id: Uuid) -> String {
    let generated = generate_link_token();
    resources
        .common
        .repos
        .email_verification
        .store_token(user_id, &generated.selector, &generated.verifier_hash, 60)
        .await
        .unwrap();
    let response =
        AxumTestRequest::get(&format!("/api/auth/verify-email?token={}", generated.token))
            .send(auth_routes(resources))
            .await;
    response
        .header("location")
        .expect("verify-email redirects")
        .to_owned()
}

#[tokio::test]
async fn an_unproven_sign_up_waits_and_is_told_to_confirm_its_address() {
    let resources = self_activating().await;
    let (status, body) = register(auth_routes(&resources), "unproven@example.com", None).await;
    assert_eq!(status, 201);
    assert_eq!(
        body["user_status"], "pending",
        "nobody proved the address yet"
    );
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("Confirm your email address"),
        "the sign-up is told what activates it, not to wait for an admin: {body}"
    );
}

#[tokio::test]
async fn following_the_confirmation_link_activates_the_account() {
    let resources = self_activating().await;
    let (_, body) = register(auth_routes(&resources), "proven@example.com", None).await;
    let user_id = Uuid::parse_str(body["user_id"].as_str().unwrap()).unwrap();

    let location = follow_confirmation_link(&resources, user_id).await;
    assert!(location.contains("status=verified"), "{location}");
    assert!(!location.contains("verified_pending"), "{location}");
    assert_eq!(
        status_of(&resources, "proven@example.com").await,
        UserStatus::Active
    );
}

#[tokio::test]
async fn without_self_activation_a_proven_address_still_waits_for_review() {
    let resources = context().await;
    let (_, body) = register(auth_routes(&resources), "reviewed@example.com", None).await;
    let user_id = Uuid::parse_str(body["user_id"].as_str().unwrap()).unwrap();

    let location = follow_confirmation_link(&resources, user_id).await;
    assert!(location.contains("verified_pending"), "{location}");
    assert_eq!(
        status_of(&resources, "reviewed@example.com").await,
        UserStatus::Pending
    );
}

#[tokio::test]
async fn an_operator_made_account_stays_in_review_while_sign_up_self_activates() {
    let resources = self_activating().await;
    let response = resources
        .account_service()
        .register(RegisterRequest {
            email: "operator-pending@example.com".to_owned(),
            password: SecretString::from(PASSWORD.to_owned()),
            display_name: None,
        })
        .await
        .unwrap();
    assert_eq!(response.user_status, UserStatus::Pending);
    assert!(
        response.message.contains("pending admin approval"),
        "the operator's door sends no confirmation link, so it is not told to confirm one: {}",
        response.message
    );
}

async fn login(resources: &ServerContext, email: &str) -> Value {
    let request: LoginRequest =
        serde_json::from_value(json!({ "email": email, "password": PASSWORD })).unwrap();
    let response = resources.account_service().login(request).await.unwrap();
    serde_json::to_value(response.user).unwrap()
}

#[tokio::test]
async fn an_address_proven_before_sign_up_opened_activates_at_the_next_sign_in() {
    let resources = context().await;
    let (_, body) = register(auth_routes(&resources), "early@example.com", None).await;
    let user_id = Uuid::parse_str(body["user_id"].as_str().unwrap()).unwrap();
    resources
        .common
        .repos
        .email_verification
        .mark_verified(user_id)
        .await
        .unwrap();

    set_policy(
        &resources,
        SignupPolicyUpdate {
            activate_on_verified_email: Some(true),
            ..Default::default()
        },
    )
    .await;
    login(&resources, "early@example.com").await;
    assert_eq!(
        status_of(&resources, "early@example.com").await,
        UserStatus::Active
    );
}

#[tokio::test]
async fn an_unproven_address_never_activates_at_sign_in() {
    let resources = self_activating().await;
    register(auth_routes(&resources), "never-proven@example.com", None).await;
    login(&resources, "never-proven@example.com").await;
    assert_eq!(
        status_of(&resources, "never-proven@example.com").await,
        UserStatus::Pending
    );
}

// ---------------------------------------------------------------------------
// 6. The admin settings route
// ---------------------------------------------------------------------------

fn admin_api(resources: &Arc<ServerContext>, human_check_configured: bool) -> Router {
    let mut context = AdminApiContext::new(AdminApiContextInit {
        database: resources.agent.database.clone(),
        repos: resources.common.repos.clone(),
        jwt_secret: resources.auth.admin_jwt_secret.to_string(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
        harness_config_registry: resources.fitness.harness_config_registry.clone(),
        guardian_config_registry: resources.fitness.guardian_config_registry.clone(),
        prompt_registry: resources.mcp.prompt_registry.clone(),
        tool_description_registry: resources.mcp.tool_description_registry.clone(),
        evidence_registry: resources.mcp.evidence_registry.clone(),
        messaging_strings_registry: resources.mcp.messaging_strings_registry.clone(),
        cageux_config_registry: resources.fitness.cageux_config_registry.clone(),
        persona_contract_registry: resources.fitness.persona_contract_registry.clone(),
        training_catalogue_registry: resources.mcp.training_catalogue_registry.clone(),
        contremaitre_config: None,
        app_behavior: resources.common.config.app_behavior.clone(),
    });
    context.human_check_configured = human_check_configured;
    AdminRoutes::routes(context)
}

async fn admin_bearer(resources: &Arc<ServerContext>, permissions: Vec<AdminPermission>) -> String {
    let token = resources
        .common
        .repos
        .admin
        .create_token(
            &CreateAdminTokenRequest {
                service_name: "signup-settings".to_owned(),
                service_description: None,
                permissions: Some(permissions),
                expires_in_days: Some(1),
                is_super_admin: false,
                tenant_id: None,
                operator_user_id: None,
            },
            resources.auth.admin_jwt_secret.as_ref(),
            &resources.auth.jwks_manager,
        )
        .await
        .unwrap();
    format!("Bearer {}", token.jwt_token.expose_secret())
}

#[tokio::test]
async fn the_admin_route_reads_and_writes_every_knob() {
    let resources = Arc::new(context().await);
    let bearer = admin_bearer(&resources, vec![AdminPermission::ManageUsers]).await;

    let shown: Value = AxumTestRequest::get("/admin/settings/signup")
        .header("authorization", &bearer)
        .send(admin_api(&resources, true))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(shown["data"]["open"], true);
    assert_eq!(shown["data"]["activate_on_verified_email"], false);
    assert_eq!(shown["data"]["turnstile_configured"], true);
    assert!(shown["data"]["created_last_24h"].is_i64());

    let updated: Value = AxumTestRequest::put("/admin/settings/signup")
        .header("authorization", &bearer)
        .json(&json!({ "open": false, "daily_limit": 0, "activate_on_verified_email": true }))
        .send(admin_api(&resources, true))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(updated["data"]["open"], false);
    assert_eq!(updated["data"]["daily_limit"], 0);
    assert_eq!(updated["data"]["activate_on_verified_email"], true);

    // A partial write leaves the other knobs as stored.
    let partial: Value = AxumTestRequest::put("/admin/settings/signup")
        .header("authorization", &bearer)
        .json(&json!({ "open": true }))
        .send(admin_api(&resources, true))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(partial["data"]["open"], true);
    assert_eq!(partial["data"]["daily_limit"], 0);
    assert_eq!(partial["data"]["activate_on_verified_email"], true);
}

#[tokio::test]
async fn the_admin_route_refuses_an_out_of_range_ceiling_and_a_token_without_manage_users() {
    let resources = Arc::new(context().await);
    let bearer = admin_bearer(&resources, vec![AdminPermission::ManageUsers]).await;
    let response = AxumTestRequest::put("/admin/settings/signup")
        .header("authorization", &bearer)
        .json(&json!({ "daily_limit": -1 }))
        .send(admin_api(&resources, false))
        .await;
    assert_eq!(response.status(), 400);

    let reader = admin_bearer(&resources, vec![AdminPermission::ListKeys]).await;
    let response = AxumTestRequest::put("/admin/settings/signup")
        .header("authorization", &reader)
        .json(&json!({ "open": false }))
        .send(admin_api(&resources, false))
        .await;
    assert_eq!(response.status(), 403);
    assert_eq!(
        signup_config(auth_routes(&resources)).await["open"],
        true,
        "a refused write changes nothing"
    );
}
