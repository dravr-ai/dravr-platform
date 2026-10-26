// ABOUTME: Signup codes end to end: an admin's or a coach's code approves whoever enters it, at registration or from the waiting screen
// ABOUTME: Drives the real register, redeem, groups and admin routes over one database (carnet#1128), plus the daily-count reset (carnet#1131)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! What a signup code does, and what it never does:
//!
//! 1. a valid code admits a registration and activates the account in its
//!    minter's name once the address is proven; an invalid one creates nothing;
//! 2. a code gets past the daily ceiling but not past a closed kill switch;
//! 3. a used-up, revoked or expired code is refused like an unknown one, and
//!    the last use of a code is claimed once;
//! 4. the waiting screen redeems a code for an account already Pending, and
//!    refuses a suspended account, a deleted account's address, a second
//!    code, and too many attempts from one address;
//! 5. a coach's code is a group invite only a roster manager can mint, within
//!    its caps, and it also joins the coach's group;
//! 6. the admin routes mint, list and revoke codes, and reset the daily count.

use crate::common;
use crate::helpers;

use std::iter;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use axum::http::StatusCode;
use axum::Router;
use chrono::{Duration, Utc};
use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use helpers::notify_capture::{capture_logs_and_spans, capture_notify, named};
use pierre_config::constants::oauth_rate_limiting::SIGNUP_CODE_ATTEMPTS;
use pierre_core::admin::models::{AdminPermission, CreateAdminTokenRequest};
use pierre_core::models::agents::CreateAgentRequest;
use pierre_core::models::{CreateGroupRequest, TenantId, User, UserStatus};
use pierre_database::database::system_settings::SETTING_SIGNUP_DAILY_LIMIT_RESET_AT;
use pierre_database::repositories::DeletionActor;
use pierre_groups::strategies::tier::EnterpriseTierStrategy;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit, AdminRoutes};
use pierre_routes_auth::AuthRoutes;
use pierre_routes_groups::GroupRoutes;
use pierre_services::signup_codes::{self, hash_code, normalize_code, MintRequest};
use pierre_services::signup_policy::{set_signup_policy, SignupPolicyUpdate, SIGNUP_PAUSED};
use pierre_services::user_removal::remove_user;
use secrecy::ExposeSecret;
use serde_json::{json, Value};
use uuid::Uuid;

const PASSWORD: &str = "securePassword123";
const INVALID: &str = "This code isn't valid";

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn auth_routes_from(resources: &ServerContext, client: Ipv4Addr) -> Router {
    AuthRoutes::routes(resources.auth_routes_context())
        .layer(MockConnectInfo(SocketAddr::new(IpAddr::V4(client), 40_000)))
}

/// The auth routes behind a fresh client address per call, so the per-address
/// sign-up window (a few a minute) never decides a test about codes.
fn auth_routes(resources: &ServerContext) -> Router {
    static NEXT: AtomicU16 = AtomicU16::new(1);
    let [hi, lo] = NEXT.fetch_add(1, Ordering::Relaxed).to_be_bytes();
    auth_routes_from(resources, Ipv4Addr::new(198, 18, hi, lo))
}

async fn register(router: Router, email: &str, code: Option<&str>) -> (u16, Value) {
    let mut body = json!({ "email": email, "password": PASSWORD });
    if let Some(code) = code {
        body["signup_code"] = json!(code);
    }
    let response = AxumTestRequest::post("/api/auth/register")
        .json(&body)
        .send(router)
        .await;
    let status = response.status();
    (
        status,
        serde_json::from_str(&response.body_text()).unwrap_or(Value::Null),
    )
}

async fn redeem(
    router: Router,
    user: &User,
    resources: &ServerContext,
    code: &str,
) -> (u16, Value) {
    let jwt = resources
        .auth
        .auth_manager
        .generate_token(user, &resources.auth.jwks_manager)
        .unwrap();
    let response = AxumTestRequest::post("/api/user/signup-code/redeem")
        .header("Authorization", &format!("Bearer {jwt}"))
        .json(&json!({ "code": code }))
        .send(router)
        .await;
    let status = response.status();
    (
        status,
        serde_json::from_str(&response.body_text()).unwrap_or(Value::Null),
    )
}

async fn account(resources: &ServerContext, email: &str) -> User {
    resources
        .common
        .repos
        .users
        .get_by_email(email)
        .await
        .unwrap()
        .expect("the account exists")
}

/// An admin's code, minted the way both admin surfaces mint one.
async fn mint(resources: &ServerContext, by: Option<Uuid>, max_uses: Option<i32>) -> String {
    signup_codes::mint(
        &resources.common.repos,
        by,
        &MintRequest {
            note: Some("test cohort".to_owned()),
            prefix: Some("beta".to_owned()),
            max_uses,
            expires_in_days: Some(7),
        },
    )
    .await
    .unwrap()
    .code
}

/// Prove the account's address as the confirmation link does.
async fn confirm_email(resources: &ServerContext, user_id: Uuid) -> UserStatus {
    resources
        .common
        .repos
        .email_verification
        .mark_verified(user_id)
        .await
        .unwrap();
    resources
        .auth_routes_context()
        .account_service()
        .apply_approval_after_verification(user_id, true)
        .await
        .unwrap()
}

async fn set_policy(resources: &ServerContext, update: SignupPolicyUpdate) {
    set_signup_policy(&resources.agent.database, update)
        .await
        .unwrap();
}

async fn minter(resources: &Arc<ServerContext>, email: &str) -> Uuid {
    create_test_user_with_plan(&resources.agent.database, email, "starter")
        .await
        .unwrap()
        .0
}

// ---------------------------------------------------------------------------
// 1. Registration with a code
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_admin_code_at_registration_activates_in_the_minters_name_once_the_address_is_proven() {
    let resources = create_test_server_resources().await.unwrap();
    let admin = minter(&resources, "minting-admin@example.com").await;
    let code = mint(&resources, Some(admin), None).await;

    let (plain, body) = register(auth_routes(&resources), "no-code@example.com", None).await;
    assert_eq!(plain, 201);
    assert_eq!(
        body["user_status"], "pending",
        "the review posture holds without a code"
    );

    // Case and separators do not matter.
    let typed = code.to_lowercase().replace('-', " ");
    let (status, body) = register(auth_routes(&resources), "coded@example.com", Some(&typed)).await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(
        body["user_status"], "pending",
        "a code vouches for the cohort, not the inbox: {body}"
    );
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("Confirm your email"));

    let user = account(&resources, "coded@example.com").await;
    let redemption = resources
        .common
        .repos
        .signup_codes
        .redemption_for(user.id)
        .await
        .unwrap()
        .expect("the code is recorded against the account");
    assert!(redemption.activated_at.is_none());

    assert_eq!(confirm_email(&resources, user.id).await, UserStatus::Active);
    let user = account(&resources, "coded@example.com").await;
    assert_eq!(user.user_status, UserStatus::Active);
    assert_eq!(
        user.approved_by,
        Some(admin),
        "approved in the minter's name"
    );
    assert!(resources
        .common
        .repos
        .signup_codes
        .redemption_for(user.id)
        .await
        .unwrap()
        .unwrap()
        .activated_at
        .is_some());
}

#[tokio::test]
async fn an_invalid_code_fails_the_registration_and_creates_nothing() {
    let resources = create_test_server_resources().await.unwrap();
    let (status, body) = register(
        auth_routes(&resources),
        "typo@example.com",
        Some("BETA-NOPE-NOPE-NOPE"),
    )
    .await;
    assert_eq!(status, 400);
    assert!(body.to_string().contains(INVALID), "{body}");
    assert_eq!(
        body["details"]["reason"], "invalid_signup_code",
        "a client words the refusal from its reason, not the English message: {body}"
    );
    assert!(resources
        .common
        .repos
        .users
        .get_by_email("typo@example.com")
        .await
        .unwrap()
        .is_none());
}

// ---------------------------------------------------------------------------
// 2. The operator's bounds
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_code_gets_past_the_daily_ceiling_but_not_past_a_closed_switch() {
    let resources = create_test_server_resources().await.unwrap();
    let code = mint(&resources, None, None).await;
    // One account in the window, so a limit of the count is a real ceiling
    // (zero would set none).
    register(auth_routes(&resources), "already-in@example.com", None).await;
    let existing = resources
        .common
        .repos
        .users
        .count_created_since(Utc::now() - Duration::hours(24))
        .await
        .unwrap();
    set_policy(
        &resources,
        SignupPolicyUpdate {
            daily_limit: Some(existing),
            ..Default::default()
        },
    )
    .await;

    let (refused, _) = register(auth_routes(&resources), "over@example.com", None).await;
    assert_eq!(refused, 429, "the ceiling is reached");
    let (admitted, body) = register(
        auth_routes(&resources),
        "with-code@example.com",
        Some(&code),
    )
    .await;
    assert_eq!(
        admitted, 201,
        "the code's own limit sized the cohort: {body}"
    );

    set_policy(
        &resources,
        SignupPolicyUpdate {
            open: Some(false),
            ..Default::default()
        },
    )
    .await;
    let (closed, body) = register(auth_routes(&resources), "closed@example.com", Some(&code)).await;
    assert_eq!(closed, 429, "the kill switch is the emergency brake");
    assert!(body.to_string().contains(SIGNUP_PAUSED), "{body}");
}

// ---------------------------------------------------------------------------
// 3. Used up, revoked, expired, and the last use
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_used_up_revoked_or_expired_code_is_refused_like_an_unknown_one() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;

    let single = mint(&resources, None, Some(1)).await;
    assert_eq!(
        register(auth_routes(&resources), "first@example.com", Some(&single))
            .await
            .0,
        201
    );
    let (status, body) =
        register(auth_routes(&resources), "second@example.com", Some(&single)).await;
    assert_eq!(status, 400, "used up: {body}");
    assert!(body.to_string().contains(INVALID));

    let revoked = signup_codes::mint(repos, None, &MintRequest::default())
        .await
        .unwrap();
    assert!(repos
        .signup_codes
        .revoke(revoked.record.id, Utc::now())
        .await
        .unwrap());
    let (status, body) = register(
        auth_routes(&resources),
        "revoked@example.com",
        Some(&revoked.code),
    )
    .await;
    assert_eq!(status, 400, "revoked: {body}");
    assert!(body.to_string().contains(INVALID));

    let expired = "EXPIREDCODE1";
    repos
        .signup_codes
        .mint(
            &hash_code(&normalize_code(expired)),
            "ODE1",
            None,
            None,
            Some(Utc::now() - Duration::minutes(1)),
            None,
        )
        .await
        .unwrap();
    let (status, body) = register(
        auth_routes(&resources),
        "expired@example.com",
        Some(expired),
    )
    .await;
    assert_eq!(status, 400, "expired: {body}");
    assert!(body.to_string().contains(INVALID));
}

#[tokio::test]
async fn the_last_use_of_a_code_is_claimed_once() {
    let resources = create_test_server_resources().await.unwrap();
    let code = mint(&resources, None, Some(1)).await;
    let repos = &resources.common.repos;
    let (a, b) = tokio::join!(
        signup_codes::claim(repos, &code),
        signup_codes::claim(repos, &code)
    );
    let claimed = [a.unwrap(), b.unwrap()]
        .iter()
        .filter(|c| c.is_some())
        .count();
    assert_eq!(claimed, 1, "two claims on the last use admit exactly one");
}

// ---------------------------------------------------------------------------
// 4. The waiting screen
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_waiting_screen_activates_a_proven_pending_account_at_once() {
    let resources = create_test_server_resources().await.unwrap();
    let admin = minter(&resources, "screen-admin@example.com").await;
    let code = mint(&resources, Some(admin), None).await;
    register(auth_routes(&resources), "waiting@example.com", None).await;
    let user = account(&resources, "waiting@example.com").await;
    resources
        .common
        .repos
        .email_verification
        .mark_verified(user.id)
        .await
        .unwrap();

    let (status, body) = redeem(auth_routes(&resources), &user, &resources, &code).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["user_status"], "active");
    assert_eq!(
        body["group_joined"],
        Value::Null,
        "an admin's code joins no group"
    );
    let user = account(&resources, "waiting@example.com").await;
    assert_eq!(user.user_status, UserStatus::Active);
    assert_eq!(user.approved_by, Some(admin));
    let tokens = resources
        .common
        .repos
        .user_mcp_tokens
        .list_tokens(user.id)
        .await
        .unwrap();
    assert_eq!(
        tokens.len(),
        1,
        "activation provisions the default MCP token, as an admin's approval does"
    );

    let (again, body) = redeem(auth_routes(&resources), &user, &resources, &code).await;
    assert_eq!(
        again, 400,
        "an active account has nothing to redeem: {body}"
    );
}

#[tokio::test]
async fn the_waiting_screen_records_a_code_until_the_address_is_proven_and_takes_one_only() {
    let resources = create_test_server_resources().await.unwrap();
    let code = mint(&resources, None, None).await;
    let other = mint(&resources, None, None).await;
    register(auth_routes(&resources), "unproven@example.com", None).await;
    let user = account(&resources, "unproven@example.com").await;

    let (status, body) = redeem(auth_routes(&resources), &user, &resources, &code).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["user_status"], "pending");

    let (second, body) = redeem(auth_routes(&resources), &user, &resources, &other).await;
    assert_eq!(second, 400, "one code per account: {body}");
    assert!(body.to_string().contains("already used a signup code"));

    assert_eq!(confirm_email(&resources, user.id).await, UserStatus::Active);
}

#[tokio::test]
async fn a_suspended_account_or_a_deleted_accounts_address_cannot_redeem() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let code = mint(&resources, None, None).await;

    register(auth_routes(&resources), "suspended@example.com", None).await;
    let suspended = account(&resources, "suspended@example.com").await;
    repos
        .users
        .update_status(suspended.id, UserStatus::Suspended, None)
        .await
        .unwrap();
    let (status, _) = redeem(auth_routes(&resources), &suspended, &resources, &code).await;
    assert_ne!(
        status, 200,
        "suspension is reversed explicitly, never by a code"
    );
    assert_eq!(
        account(&resources, "suspended@example.com")
            .await
            .user_status,
        UserStatus::Suspended
    );

    // A deleted account tombstones its address.
    register(auth_routes(&resources), "returning@example.com", None).await;
    let deleted = account(&resources, "returning@example.com").await;
    remove_user(repos, None, None, None, deleted.id, DeletionActor::Athlete)
        .await
        .unwrap();
    register(auth_routes(&resources), "returning@example.com", None).await;
    let newcomer = account(&resources, "returning@example.com").await;
    repos
        .email_verification
        .mark_verified(newcomer.id)
        .await
        .unwrap();
    let (status, body) = redeem(auth_routes(&resources), &newcomer, &resources, &code).await;
    assert_eq!(
        status, 400,
        "a code never overrides a deleted account's hold: {body}"
    );
    assert_eq!(
        account(&resources, "returning@example.com")
            .await
            .user_status,
        UserStatus::Pending
    );
}

#[tokio::test]
async fn too_many_code_attempts_from_one_address_are_refused() {
    let resources = create_test_server_resources().await.unwrap();
    register(auth_routes(&resources), "guesser@example.com", None).await;
    let user = account(&resources, "guesser@example.com").await;
    let client = Ipv4Addr::new(198, 51, 100, 99);
    let limit = SIGNUP_CODE_ATTEMPTS;
    for attempt in 0..limit {
        let (status, _) = redeem(
            auth_routes_from(&resources, client),
            &user,
            &resources,
            &format!("WRONG{attempt}"),
        )
        .await;
        assert_eq!(status, 400, "attempt {attempt} is answered as a bad code");
    }
    let (status, _) = redeem(
        auth_routes_from(&resources, client),
        &user,
        &resources,
        "WRONGAGAIN",
    )
    .await;
    assert_eq!(status, 429, "past the window, the address is told to wait");
}

// ---------------------------------------------------------------------------
// 5. A coach's code
// ---------------------------------------------------------------------------

/// A Professional-plan owner of one group, the groups router, and their auth.
async fn coach_with_group(
    resources: &Arc<ServerContext>,
    manages_roster: bool,
) -> (Uuid, String, String, Router) {
    let (owner_id, owner, _) = create_test_user_with_plan(
        &resources.agent.database,
        &format!("coach-{}@example.com", Uuid::new_v4()),
        "professional",
    )
    .await
    .unwrap();
    resources
        .common
        .repos
        .users
        .set_manages_roster(owner_id, manages_roster)
        .await
        .unwrap();
    let tenant: TenantId = resources
        .common
        .repos
        .tenants
        .list_for_user(owner_id)
        .await
        .unwrap()
        .first()
        .unwrap()
        .id;
    let agent: CreateAgentRequest = serde_json::from_value(
        json!({"title":"Coach","system_prompt":"Test.","category":"training","tags":["run"]}),
    )
    .unwrap();
    let agent_id = resources
        .common
        .repos
        .agents
        .create(owner_id, tenant, &agent)
        .await
        .unwrap()
        .id
        .to_string();
    let group = resources
        .group_service()
        .create_group(
            &CreateGroupRequest {
                name: "Coded Group".to_owned(),
                description: None,
                agent_id,
                max_members: Some(10),
                coach_user_id: None,
            },
            owner_id,
            tenant,
            &EnterpriseTierStrategy,
        )
        .await
        .unwrap();
    let auth = format!("Bearer {}", generate_test_token(resources, &owner).await);
    (
        owner_id,
        group.id.to_string(),
        auth,
        GroupRoutes::routes(Arc::clone(resources)),
    )
}

async fn create_invite(router: &Router, auth: &str, group_id: &str, body: Value) -> (u16, Value) {
    let response = AxumTestRequest::post(&format!("/api/groups/{group_id}/invites"))
        .header("authorization", auth)
        .json(&body)
        .send(router.clone())
        .await;
    let status = response.status();
    (
        status,
        serde_json::from_str(&response.body_text()).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn a_coachs_code_approves_the_account_and_joins_the_coachs_group() {
    let resources = create_test_server_resources().await.unwrap();
    let (coach, group_id, auth, router) = coach_with_group(&resources, true).await;
    let (status, invite) = create_invite(
        &router,
        &auth,
        &group_id,
        json!({ "approves_signup": true, "max_uses": 5, "expires_in_days": 14 }),
    )
    .await;
    assert_eq!(status, 201, "{invite}");
    assert_eq!(invite["approves_signup"], true);
    let code = invite["code"].as_str().unwrap().to_owned();
    let (_, plain) = create_invite(&router, &auth, &group_id, json!({})).await;
    assert!(
        code.len() > plain["code"].as_str().unwrap().len(),
        "an approving invite carries a longer code than a plain one: {code}"
    );

    let (status, body) =
        register(auth_routes(&resources), "athlete@example.com", Some(&code)).await;
    assert_eq!(status, 201, "{body}");
    let athlete = account(&resources, "athlete@example.com").await;
    assert!(
        resources
            .common
            .repos
            .groups
            .get_member(&group_id, athlete.id)
            .await
            .unwrap()
            .is_some(),
        "the coach's code joins the coach's group"
    );

    assert_eq!(
        confirm_email(&resources, athlete.id).await,
        UserStatus::Active
    );
    assert_eq!(
        account(&resources, "athlete@example.com").await.approved_by,
        Some(coach),
        "approved in the coach's name"
    );

    // The admin listing shows the coach's code next to admins' codes.
    let listed = signup_codes::list(&resources.common.repos).await.unwrap();
    let coach_code = listed
        .iter()
        .find(|c| c.kind == signup_codes::CodeKind::Coach)
        .expect("the coach's code is listed");
    assert_eq!(coach_code.use_count, 1);
    assert_eq!(coach_code.redeemed_by.len(), 1);
}

/// An address an operator already allowed registers active on its own; the
/// coach's code it also carries must still join the coach's group, and count
/// as used, rather than being refused as "already active" and dropped.
#[tokio::test]
async fn a_coachs_code_still_joins_the_group_when_the_address_was_already_allowed() {
    let resources = create_test_server_resources().await.unwrap();
    let (_, group_id, auth, router) = coach_with_group(&resources, true).await;
    let (status, invite) = create_invite(
        &router,
        &auth,
        &group_id,
        json!({ "approves_signup": true, "max_uses": 5, "expires_in_days": 14 }),
    )
    .await;
    assert_eq!(status, 201, "{invite}");
    let code = invite["code"].as_str().unwrap().to_owned();
    let email = "allowed-athlete@example.com";
    resources
        .common
        .repos
        .pre_approved_emails
        .allow(email, None, Some("cohort"))
        .await
        .unwrap();

    let (status, body) = register(auth_routes(&resources), email, Some(&code)).await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(
        body["activation"], "active",
        "the allow activates it: {body}"
    );
    let athlete = account(&resources, email).await;
    assert_eq!(
        athlete.user_status,
        UserStatus::Active,
        "the allowed address is active at once"
    );
    assert!(
        resources
            .common
            .repos
            .groups
            .get_member(&group_id, athlete.id)
            .await
            .unwrap()
            .is_some(),
        "the coach's code joins the coach's group even though the allow already activated the account"
    );
    let listed = signup_codes::list(&resources.common.repos).await.unwrap();
    let coach_code = listed
        .iter()
        .find(|c| c.kind == signup_codes::CodeKind::Coach)
        .expect("the coach's code is listed");
    assert_eq!(
        coach_code.use_count, 1,
        "the code counts the use it was put to"
    );
}

#[tokio::test]
async fn only_a_roster_manager_mints_a_coach_code_and_only_within_its_caps() {
    let resources = create_test_server_resources().await.unwrap();
    let (_, group_id, auth, router) = coach_with_group(&resources, false).await;
    let (status, _) = create_invite(
        &router,
        &auth,
        &group_id,
        json!({ "approves_signup": true, "max_uses": 5, "expires_in_days": 14 }),
    )
    .await;
    assert_eq!(
        status, 403,
        "a group owner without coach access cannot mint one"
    );
    let (status, invite) = create_invite(&router, &auth, &group_id, json!({})).await;
    assert_eq!(status, 201, "a plain invite needs nothing more");
    assert_eq!(invite["approves_signup"], false);

    let (_, group_id, auth, router) = coach_with_group(&resources, true).await;
    for body in [
        json!({ "approves_signup": true }),
        json!({ "approves_signup": true, "max_uses": 51, "expires_in_days": 14 }),
        json!({ "approves_signup": true, "max_uses": 5, "expires_in_days": 31 }),
        json!({ "approves_signup": true, "max_uses": 5, "expires_in_days": 14, "kind": "coach" }),
    ] {
        let (status, response) = create_invite(&router, &auth, &group_id, body.clone()).await;
        assert_eq!(status, 400, "{body} → {response}");
    }

    // A plain member invite never approves anyone.
    let (_, plain) = create_invite(&router, &auth, &group_id, json!({})).await;
    let (status, _) = register(
        auth_routes(&resources),
        "plain-invite@example.com",
        Some(plain["code"].as_str().unwrap()),
    )
    .await;
    assert_eq!(status, 400, "a group invite is not a signup code");
}

// ---------------------------------------------------------------------------
// 6. The admin routes
// ---------------------------------------------------------------------------

fn admin_api(resources: &Arc<ServerContext>) -> Router {
    AdminRoutes::routes(AdminApiContext::new(AdminApiContextInit {
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
    }))
}

async fn admin_bearer(resources: &Arc<ServerContext>, permissions: Vec<AdminPermission>) -> String {
    let token = resources
        .common
        .repos
        .admin
        .create_token(
            &CreateAdminTokenRequest {
                service_name: "signup-codes".to_owned(),
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
async fn the_admin_routes_mint_once_list_by_hint_and_revoke() {
    let resources = create_test_server_resources().await.unwrap();
    let bearer = admin_bearer(&resources, vec![AdminPermission::ManageUsers]).await;

    let minted: Value = AxumTestRequest::post("/admin/signup-codes")
        .header("authorization", &bearer)
        .json(&json!({ "note": "launch talk", "prefix": "talk", "max_uses": 20 }))
        .send(admin_api(&resources))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let code = minted["data"]["code"].as_str().unwrap().to_owned();
    assert!(code.starts_with("TALK-"), "{code}");
    let id = minted["data"]["record"]["id"].as_str().unwrap().to_owned();

    let listed = AxumTestRequest::get("/admin/signup-codes")
        .header("authorization", &bearer)
        .send(admin_api(&resources))
        .await
        .assert_status(StatusCode::OK);
    let text = listed.body_text();
    assert!(
        !text.contains(&code),
        "the code is shown once, never listed"
    );
    let listed: Value = serde_json::from_str(&text).unwrap();
    let entry = &listed["data"]["codes"][0];
    assert_eq!(entry["kind"], "admin");
    assert_eq!(
        entry["code_hint"],
        &normalize_code(&code)[normalize_code(&code).len() - 4..]
    );
    assert_eq!(entry["note"], "launch talk");
    assert_eq!(entry["live"], true);

    AxumTestRequest::delete(&format!("/admin/signup-codes/admin/{id}"))
        .header("authorization", &bearer)
        .send(admin_api(&resources))
        .await
        .assert_status(StatusCode::OK);
    let (status, _) = register(
        auth_routes(&resources),
        "after-revoke@example.com",
        Some(&code),
    )
    .await;
    assert_eq!(status, 400, "a revoked code approves no one");

    let unknown = AxumTestRequest::delete(&format!("/admin/signup-codes/admin/{}", Uuid::new_v4()))
        .header("authorization", &bearer)
        .send(admin_api(&resources))
        .await;
    assert_eq!(
        unknown.status(),
        404,
        "an id that names no code is not 'already revoked'"
    );

    let reader = admin_bearer(&resources, vec![AdminPermission::ListKeys]).await;
    let response = AxumTestRequest::post("/admin/signup-codes")
        .header("authorization", &reader)
        .json(&json!({}))
        .send(admin_api(&resources))
        .await;
    assert_eq!(response.status(), 403, "minting needs ManageUsers");
}

#[tokio::test]
async fn the_reset_restarts_the_daily_count_and_keeps_the_limit() {
    let resources = create_test_server_resources().await.unwrap();
    let bearer = admin_bearer(&resources, vec![AdminPermission::ManageUsers]).await;
    register(auth_routes(&resources), "before-reset@example.com", None).await;
    let existing = resources
        .common
        .repos
        .users
        .count_created_since(Utc::now() - Duration::hours(24))
        .await
        .unwrap();
    set_policy(
        &resources,
        SignupPolicyUpdate {
            daily_limit: Some(existing),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(
        register(auth_routes(&resources), "refused@example.com", None)
            .await
            .0,
        429,
        "the ceiling is reached"
    );

    let reset: Value = AxumTestRequest::post("/admin/settings/signup/reset")
        .header("authorization", &bearer)
        .json(&json!({}))
        .send(admin_api(&resources))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(reset["data"]["counted"], 0, "{reset}");
    assert_eq!(
        reset["data"]["daily_limit"], existing,
        "the limit is unchanged"
    );
    assert!(reset["data"]["counted_since"].is_string());

    assert_eq!(
        register(auth_routes(&resources), "after-reset@example.com", None)
            .await
            .0,
        201,
        "the count starts over from the reset"
    );
    let shown: Value = AxumTestRequest::get("/admin/settings/signup")
        .header("authorization", &bearer)
        .send(admin_api(&resources))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(shown["data"]["counted"], 1);

    let reader = admin_bearer(&resources, vec![AdminPermission::ListKeys]).await;
    let response = AxumTestRequest::post("/admin/settings/signup/reset")
        .header("authorization", &reader)
        .json(&json!({}))
        .send(admin_api(&resources))
        .await;
    assert_eq!(response.status(), 403, "resetting needs ManageUsers");
}

#[tokio::test]
async fn an_admin_revokes_a_coachs_code() {
    let resources = create_test_server_resources().await.unwrap();
    let bearer = admin_bearer(&resources, vec![AdminPermission::ManageUsers]).await;
    let (_, group_id, auth, router) = coach_with_group(&resources, true).await;
    let (_, invite) = create_invite(
        &router,
        &auth,
        &group_id,
        json!({ "approves_signup": true, "max_uses": 5, "expires_in_days": 14 }),
    )
    .await;
    let code = invite["code"].as_str().unwrap().to_owned();
    let id = invite["id"].as_str().unwrap().to_owned();

    let revoked: Value = AxumTestRequest::delete(&format!("/admin/signup-codes/coach/{id}"))
        .header("authorization", &bearer)
        .send(admin_api(&resources))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(revoked["data"]["revoked"], true, "{revoked}");

    let (status, body) =
        register(auth_routes(&resources), "too-late@example.com", Some(&code)).await;
    assert_eq!(
        status, 400,
        "a revoked coach's code approves no one: {body}"
    );
    let listed = signup_codes::list(&resources.common.repos).await.unwrap();
    let entry = listed
        .iter()
        .find(|c| c.id.to_string() == id)
        .expect("the revoked code is still listed");
    assert!(!entry.live, "a revoked coach's code is listed as not live");
}

#[tokio::test]
async fn a_reset_older_than_a_day_changes_nothing() {
    let resources = create_test_server_resources().await.unwrap();
    let bearer = admin_bearer(&resources, vec![AdminPermission::ManageUsers]).await;
    register(auth_routes(&resources), "inside-the-day@example.com", None).await;
    let day_count = resources
        .common
        .repos
        .users
        .count_created_since(Utc::now() - Duration::hours(24))
        .await
        .unwrap();
    assert!(day_count >= 1, "fixture precondition");
    resources
        .agent
        .database
        .set_system_setting(
            SETTING_SIGNUP_DAILY_LIMIT_RESET_AT,
            &(Utc::now() - Duration::hours(25)).to_rfc3339(),
        )
        .await
        .unwrap();

    let shown: Value = AxumTestRequest::get("/admin/settings/signup")
        .header("authorization", &bearer)
        .send(admin_api(&resources))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(
        shown["data"]["counted"], day_count,
        "a reset before the window opened leaves the full 24 hours counted: {shown}"
    );
}

#[tokio::test]
async fn a_code_never_reaches_the_logs() {
    let resources = create_test_server_resources().await.unwrap();
    let at_registration = mint(&resources, None, None).await;
    let on_the_waiting_screen = mint(&resources, None, None).await;
    register(auth_routes(&resources), "logged-waiting@example.com", None).await;
    let waiting = account(&resources, "logged-waiting@example.com").await;

    let (logs, spans, guard) = capture_logs_and_spans();
    let (status, _) = register(
        auth_routes(&resources),
        "logged-coded@example.com",
        Some(&at_registration),
    )
    .await;
    assert_eq!(status, 201);
    let (status, _) = redeem(
        auth_routes(&resources),
        &waiting,
        &resources,
        &on_the_waiting_screen,
    )
    .await;
    assert_eq!(status, 200);
    drop(guard);

    let lines = logs.lock().unwrap().clone();
    assert!(
        lines
            .iter()
            .any(|line| line.event == "Signup code redeemed"),
        "the capture saw the redemptions"
    );
    for code in [&at_registration, &on_the_waiting_screen] {
        let normalized = normalize_code(code);
        for line in &lines {
            for text in iter::once(&line.event).chain(line.fields.values()) {
                assert!(
                    !text.contains(code.as_str()) && !text.contains(&normalized),
                    "a code reached the log line {:?}",
                    line.event
                );
            }
        }
        for (field, value) in spans.lock().unwrap().iter() {
            assert!(
                !value.contains(code.as_str()) && !value.contains(&normalized),
                "a span recorded the code under {field}"
            );
        }
    }
}

#[tokio::test]
async fn a_code_at_a_deleted_accounts_address_waits_for_review_once_and_spends_no_use() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let minted = signup_codes::mint(repos, None, &MintRequest::default())
        .await
        .unwrap();
    register(auth_routes(&resources), "held@example.com", None).await;
    let deleted = account(&resources, "held@example.com").await;
    remove_user(repos, None, None, None, deleted.id, DeletionActor::Athlete)
        .await
        .unwrap();

    let (events, guard) = capture_notify();
    let (status, body) = register(
        auth_routes(&resources),
        "held@example.com",
        Some(&minted.code),
    )
    .await;
    drop(guard);

    assert_eq!(status, 201, "{body}");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("pending admin approval"),
        "a code never vouches for a deleted account's address: {body}"
    );
    assert_eq!(
        named(&events, "user.pending_approval").len(),
        1,
        "the account is announced for review once"
    );
    let newcomer = account(&resources, "held@example.com").await;
    assert!(repos
        .signup_codes
        .redemption_for(newcomer.id)
        .await
        .unwrap()
        .is_none());
    let listed = signup_codes::list(repos).await.unwrap();
    let entry = listed
        .iter()
        .find(|c| c.id == minted.record.id)
        .expect("the code is listed");
    assert_eq!(entry.use_count, 0, "a refused redemption spends no use");
}
