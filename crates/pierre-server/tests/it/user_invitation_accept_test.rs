// ABOUTME: E2e coverage for single-use invitations — issue, redeem at /api/auth/accept-invitation, sign in
// ABOUTME: Redemption lands an Active account tied to the invited address; reuse, expiry, supersession and lockout fail
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! registre#43: an operator invites an address, and the invitee chooses their
//! own password on the link the invitation mails — no password crosses a side
//! channel, and no approval queue stands between the invitee and the app.
//!
//! These drive the real `/api/auth/accept-invitation` handler and the apps'
//! sign-in over one in-memory database, with global auto-approval off and no
//! `AUTO_APPROVE_DOMAINS`, so an `Active` account here is the invitation's
//! doing. Every assertion is on content: the created account's status,
//! address, attribution and verified address, the password that signs in, and
//! the one uniform refusal a spent link answers.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;
use pierre_test_support::account::delete_account;

use chrono::{Duration, Utc};
use helpers::axum_test::AxumTestRequest;
use helpers::first_party_sign_in::SignIn;
use pierre_config::environment::{
    AppBehaviorConfig, BackupConfig, DatabaseConfig, DatabaseUrl, Environment, SecurityConfig,
    SecurityHeadersConfig, ServerConfig,
};
use pierre_core::models::UserStatus;
use pierre_mcp_server::mcp::resources::{ServerContext, ServerContextOptions};
use pierre_routes_auth::AuthRoutes;
use pierre_services::invitation::{
    self, InviteOutcome, IssuedInvitation, INVITATIONS_PER_ADDRESS_PER_HOUR, INVITATION_TTL_DAYS,
};
use pierre_services::link_token::generate_link_token;
use pierre_services::pre_approval;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

const PASSWORD: &str = "chosenByTheInvitee1";
/// What a client reads for every redemption failure: the sanitized not-found
/// answer, the same whether the link was unknown, expired, used, replaced,
/// revoked or locked out, so a response never says which.
const UNIFORM_REFUSAL: &str = "The requested resource was not found";

/// A real `ServerContext` over an in-memory `SQLite` database, with public
/// registration left in the pending → admin-approval mode.
async fn setup() -> Arc<ServerContext> {
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

    Arc::new(
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
    )
}

/// A real operator account: `users.approved_by` references `users.id`.
async fn operator(resources: &Arc<ServerContext>, email: &str) -> Uuid {
    let response = AxumTestRequest::post("/api/auth/register")
        .json(&json!({ "email": email, "password": "operatorPassword123" }))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    let status = response.status();
    let body: Value = response.json();
    assert_eq!(status, 201, "operator registration must succeed: {body}");
    Uuid::parse_str(body["user_id"].as_str().unwrap()).unwrap()
}

/// Issue an invitation through the service the admin route calls.
async fn invite(resources: &Arc<ServerContext>, email: &str, by: Option<Uuid>) -> IssuedInvitation {
    match invitation::issue(&resources.common.repos, email, by)
        .await
        .expect("issuing must succeed")
    {
        InviteOutcome::Issued(issued) => issued,
        InviteOutcome::AccountExists => panic!("{email} unexpectedly has an account"),
        InviteOutcome::BudgetSpent => panic!("{email} unexpectedly spent its budget"),
    }
}

/// POST the real acceptance handler and return `(status, body)`.
async fn accept(
    resources: &Arc<ServerContext>,
    token: &str,
    password: &str,
    display_name: Option<&str>,
) -> (u16, Value) {
    let response = AxumTestRequest::post("/api/auth/accept-invitation")
        .json(&json!({ "token": token, "password": password, "display_name": display_name }))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    let status = response.status();
    (status, response.json())
}

fn assert_uniform_refusal(status: u16, body: &Value, why: &str) {
    assert_eq!(status, 404, "{why}: {body}");
    assert_eq!(
        body["message"].as_str(),
        Some(UNIFORM_REFUSAL),
        "{why} must answer the one uniform refusal: {body}"
    );
}

#[tokio::test]
async fn redemption_creates_an_active_account_tied_to_the_invited_address() {
    let resources = setup().await;
    let operator_id = operator(&resources, "inviting-operator@example.com").await;
    let issued = invite(&resources, "Coach.Phil@Example.com", Some(operator_id)).await;
    assert_eq!(
        issued.email, "coach.phil@example.com",
        "the address is stored normalized"
    );
    assert!(
        issued.expires_at > Utc::now() + Duration::days(INVITATION_TTL_DAYS) - Duration::minutes(1)
            && issued.expires_at <= Utc::now() + Duration::days(INVITATION_TTL_DAYS),
        "the link must live for the documented window: {}",
        issued.expires_at
    );

    let (status, body) = accept(&resources, &issued.token, PASSWORD, Some("Coach Phil")).await;
    assert_eq!(status, 201, "acceptance must create the account: {body}");
    assert_eq!(body["user_status"].as_str(), Some("active"), "{body}");
    assert_eq!(body["display_name"].as_str(), Some("Coach Phil"), "{body}");

    let repos = &resources.common.repos;
    let user = repos
        .users
        .get_by_email("coach.phil@example.com")
        .await
        .unwrap()
        .expect("the invited address must now have an account");
    assert_eq!(user.id.to_string(), body["user_id"].as_str().unwrap());
    assert_eq!(user.email, "coach.phil@example.com");
    assert_eq!(user.user_status, UserStatus::Active);
    assert_eq!(
        user.approved_by,
        Some(operator_id),
        "approved_by must attribute the inviting operator"
    );
    assert!(user.approved_at.is_some(), "approved_at must be stamped");
    assert!(
        repos.email_verification.is_verified(user.id).await.unwrap(),
        "following the mailed link proves the inbox"
    );
    assert_eq!(
        repos.tenants.list_for_user(user.id).await.unwrap().len(),
        1,
        "the account gets its personal tenant, as a registration does"
    );

    // The password the invitee chose is the one that signs in.
    let login = SignIn::new("coach.phil@example.com", PASSWORD)
        .run(&resources)
        .await
        .token();
    assert_eq!(login.status(), 200, "the chosen password must sign in");
    let login_body: Value = login.json();
    assert_eq!(
        login_body["user"]["user_status"].as_str(),
        Some("active"),
        "{login_body}"
    );
}

#[tokio::test]
async fn an_invitation_outliving_its_operator_still_creates_the_account_unattributed() {
    let resources = setup().await;
    let repos = &resources.common.repos;
    let operator_id = operator(&resources, "departed-operator@example.com").await;
    let issued = invite(&resources, "outlived@example.com", Some(operator_id)).await;

    delete_account(repos, operator_id, None)
        .await
        .expect("an operator whose only trace is an invitation can be deleted");

    let (status, body) = accept(&resources, &issued.token, PASSWORD, None).await;
    assert_eq!(
        status, 201,
        "the link must still create the account: {body}"
    );
    let user = repos
        .users
        .get_by_email("outlived@example.com")
        .await
        .unwrap()
        .expect("the invited address must now have an account");
    assert_eq!(user.user_status, UserStatus::Active);
    assert_eq!(
        user.approved_by, None,
        "a deleted operator cannot be the account's approver"
    );
}

#[tokio::test]
async fn a_second_redemption_of_the_same_token_fails() {
    let resources = setup().await;
    let issued = invite(&resources, "once-only@example.com", None).await;

    let (status, body) = accept(&resources, &issued.token, PASSWORD, None).await;
    assert_eq!(status, 201, "the first redemption must succeed: {body}");

    let (status, body) = accept(&resources, &issued.token, "anotherPassword99", None).await;
    assert_uniform_refusal(status, &body, "a redeemed link");

    // The account is the first redemption's, password included.
    let login = SignIn::new("once-only@example.com", PASSWORD)
        .run(&resources)
        .await
        .token();
    assert_eq!(
        login.status(),
        200,
        "the first redemption's password must still sign in"
    );
}

#[tokio::test]
async fn an_expired_invitation_creates_nothing() {
    let resources = setup().await;
    let repos = &resources.common.repos;
    let email = "too-late@example.com";
    let generated = generate_link_token();
    repos
        .user_invitations
        .issue(
            email,
            None,
            &generated.selector,
            &generated.verifier_hash,
            Utc::now() - Duration::minutes(1),
        )
        .await
        .unwrap();

    let (status, body) = accept(&resources, &generated.token, PASSWORD, None).await;
    assert_uniform_refusal(status, &body, "an expired link");
    assert!(
        repos.users.get_by_email(email).await.unwrap().is_none(),
        "an expired link must create no account"
    );
}

#[tokio::test]
async fn a_newer_invitation_replaces_the_older_link() {
    let resources = setup().await;
    let first = invite(&resources, "resent@example.com", None).await;
    let second = invite(&resources, "resent@example.com", None).await;

    let (status, body) = accept(&resources, &first.token, PASSWORD, None).await;
    assert_uniform_refusal(status, &body, "a superseded link");

    let (status, body) = accept(&resources, &second.token, PASSWORD, None).await;
    assert_eq!(status, 201, "the newest link must work: {body}");
}

#[tokio::test]
async fn wrong_verifiers_lock_the_invitation_after_the_attempt_cap() {
    let resources = setup().await;
    let issued = invite(&resources, "guessed-at@example.com", None).await;
    let selector = issued.token.split_once('.').unwrap().0;
    let forged = format!("{selector}.WrongVerifierWrongVerifierWrong1");

    for attempt in 1..=5 {
        let (status, body) = accept(&resources, &forged, PASSWORD, None).await;
        assert_uniform_refusal(status, &body, &format!("wrong verifier #{attempt}"));
    }

    let (status, body) = accept(&resources, &issued.token, PASSWORD, None).await;
    assert_uniform_refusal(status, &body, "the real token after the lockout");
    assert!(
        resources
            .common
            .repos
            .users
            .get_by_email("guessed-at@example.com")
            .await
            .unwrap()
            .is_none(),
        "a locked-out invitation must create no account"
    );
}

#[tokio::test]
async fn a_weak_password_is_refused_without_spending_the_invitation() {
    let resources = setup().await;
    let issued = invite(&resources, "weak-first@example.com", None).await;

    let (status, body) = accept(&resources, &issued.token, "short", None).await;
    assert_eq!(status, 400, "a short password must be refused: {body}");
    assert!(
        body.to_string().contains("at least 8 characters"),
        "the refusal must say why: {body}"
    );

    let (status, body) = accept(&resources, &issued.token, PASSWORD, None).await;
    assert_eq!(
        status, 201,
        "the invitation must still be redeemable: {body}"
    );
}

#[tokio::test]
async fn a_malformed_token_answers_the_uniform_refusal() {
    let resources = setup().await;
    for token in ["", "no-delimiter", ".verifier-only", "selector-only."] {
        let (status, body) = accept(&resources, token, PASSWORD, None).await;
        assert_uniform_refusal(status, &body, &format!("token {token:?}"));
    }
}

#[tokio::test]
async fn an_address_that_registered_meanwhile_keeps_its_own_account() {
    let resources = setup().await;
    let email = "registered-meanwhile@example.com";
    let issued = invite(&resources, email, None).await;

    let response = AxumTestRequest::post("/api/auth/register")
        .json(&json!({ "email": email, "password": "theirOwnPassword1" }))
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    assert_eq!(response.status(), 201, "self-registration must succeed");

    let (status, body) = accept(&resources, &issued.token, PASSWORD, None).await;
    assert_eq!(
        status, 403,
        "an existing account must not be overwritten: {body}"
    );

    let login = SignIn::new(email, "theirOwnPassword1")
        .run(&resources)
        .await
        .token();
    assert_eq!(
        login.status(),
        200,
        "the self-registered password must be untouched"
    );

    // Once the account exists there is nothing left to invite it to.
    assert!(matches!(
        invitation::issue(&resources.common.repos, email, None)
            .await
            .unwrap(),
        InviteOutcome::AccountExists
    ));
}

#[tokio::test]
async fn withdrawing_the_allow_revokes_the_invitation() {
    let resources = setup().await;
    let repos = &resources.common.repos;
    let email = "withdrawn@example.com";
    pre_approval::allow(repos, email, None, Some("cohort"))
        .await
        .unwrap();
    let issued = invite(&resources, email, None).await;

    let listed = pre_approval::list(repos).await.unwrap();
    assert_eq!(listed.len(), 1);
    let listed_expiry = listed[0]
        .invitation_expires_at
        .expect("the listing must show the outstanding invitation");
    // Postgres keeps microseconds, so compare to the millisecond.
    assert!(
        (listed_expiry - issued.expires_at).num_milliseconds().abs() < 1,
        "the listed expiry must be the issued one: {listed_expiry} vs {}",
        issued.expires_at
    );

    let result = pre_approval::disallow(repos, email).await.unwrap();
    assert!(result.removed);
    assert!(
        result.invitation_revoked,
        "the invitation goes with the allow"
    );
    assert!(
        result
            .message()
            .contains("its invitation link no longer works"),
        "{}",
        result.message()
    );

    let (status, body) = accept(&resources, &issued.token, PASSWORD, None).await;
    assert_uniform_refusal(status, &body, "a revoked link");
}

#[tokio::test]
async fn a_redeemed_invitation_leaves_the_outstanding_list() {
    let resources = setup().await;
    let repos = &resources.common.repos;
    let email = "listed-then-redeemed@example.com";
    pre_approval::allow(repos, email, None, None).await.unwrap();
    let issued = invite(&resources, email, None).await;

    let (status, body) = accept(&resources, &issued.token, PASSWORD, None).await;
    assert_eq!(status, 201, "{body}");

    let listed = pre_approval::list(repos).await.unwrap();
    assert_eq!(listed[0].account_status.as_deref(), Some("active"));
    assert_eq!(
        listed[0].invitation_expires_at, None,
        "a redeemed invitation is no longer outstanding"
    );
}

#[tokio::test]
async fn an_address_is_sent_a_bounded_number_of_invitations_per_hour() {
    let resources = setup().await;
    let email = "sent-too-often@example.com";
    for _ in 0..INVITATIONS_PER_ADDRESS_PER_HOUR {
        invite(&resources, email, None).await;
    }

    assert!(
        matches!(
            invitation::issue(&resources.common.repos, email, None)
                .await
                .unwrap(),
            InviteOutcome::BudgetSpent
        ),
        "the send past the hourly budget must be refused"
    );
    // Another address is unaffected.
    invite(&resources, "someone-else@example.com", None).await;
}
