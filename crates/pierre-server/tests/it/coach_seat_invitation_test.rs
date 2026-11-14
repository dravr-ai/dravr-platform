// ABOUTME: E2e coverage for the coach-seat invitation — the console issues, lists and revokes it; redemption grants the seat and the role
// ABOUTME: Only the invited address gets an account; expired and revoked links are refused; an address with an account is offered the grant
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1524: a super-admin invites a coach the team recruited. The invite
//! is the single-use mailed link of every invitation, locked to the address,
//! and its redemption also settles coach access (`manages_roster`, an
//! operator grant by the inviter) and the coach role, so the coach meets no
//! role question and no request.
//!
//! These drive the real admin routes (`/admin/coach-seat-invitations`), the
//! real acceptance handler and the real onboarding-status read over one
//! in-memory database, with global auto-approval off, so every grant, role
//! and step asserted here is the invitation's doing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;
use secrecy::ExposeSecret;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{Duration, Utc};
use helpers::axum_test::AxumTestRequest;
use pierre_auth::admin::jwks::JwksManager;
use pierre_config::environment::{
    BackupConfig, DatabaseConfig, DatabaseUrl, Environment, SecurityConfig, SecurityHeadersConfig,
    ServerConfig,
};
use pierre_config::mcp::AppBehaviorConfig;
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_core::admin::models::CreateAdminTokenRequest;
use pierre_core::models::{InvitationKind, User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::{ServerContext, ServerContextOptions};
use pierre_mcp_server::routes::onboarding::OnboardingRoutes;
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit, AdminRoutes};
use pierre_routes_auth::AuthRoutes;
use pierre_services::invitation::{self, InviteOutcome, IssuedInvitation};
use pierre_services::link_token::{generate_link_token, split_link_token};
use pierre_services::user_approval::UserApprovalNotifier;
use pierre_tool_runtime::guardian::GuardianConfigRegistry;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, PoisonError};
use uuid::Uuid;

const TEST_ADMIN_JWT_SECRET: &str = "test_jwt_secret_for_coach_seat_invitations";
const FRONTEND_URL: &str = "https://app.example.test";
const PASSWORD: &str = "chosenByTheCoach1";
/// What a client reads for every redemption failure, whichever it was.
const UNIFORM_REFUSAL: &str = "The requested resource was not found";

/// One invitation handed over to be mailed.
#[derive(Clone, Debug)]
struct MailedInvitation {
    email: String,
    token: String,
    kind: InvitationKind,
    url: String,
}

/// Records what the handler asked to be mailed.
#[derive(Default)]
struct RecordingNotifier {
    invited: Arc<Mutex<Vec<MailedInvitation>>>,
}

#[async_trait]
impl UserApprovalNotifier for RecordingNotifier {
    async fn notify_user_approved(
        &self,
        _user_id: Uuid,
        _email: &str,
        _display_name: Option<&str>,
    ) {
    }

    async fn notify_user_invited(&self, invitation: &IssuedInvitation) -> bool {
        self.invited
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(MailedInvitation {
                email: invitation.email.clone(),
                token: invitation.token.clone(),
                kind: invitation.kind,
                url: invitation.url(FRONTEND_URL),
            });
        true
    }
}

/// One in-memory database behind the admin routes, the acceptance handler and
/// the onboarding read, and the super-admin operator issuing invitations.
struct Harness {
    resources: Arc<ServerContext>,
    admin: axum::Router,
    repos: Arc<RepositoryRegistry>,
    jwks: Arc<JwksManager>,
    operator_id: Uuid,
    operator_email: String,
    invited: Arc<Mutex<Vec<MailedInvitation>>>,
}

impl Harness {
    async fn new() -> Result<Self> {
        common::init_server_config();
        let database = common::create_test_database().await?;
        let auth_manager = common::create_test_auth_manager();
        let jwks = common::get_shared_test_jwks();
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
                    jwks_manager: Some(Arc::clone(&jwks)),
                    llm_provider: None,
                    chat_provider: None,
                    extra_tools: Vec::new(),
                    billing_provider: None,
                    turn_runner: None,
                    archive_imports: None,
                    live_records: None,
                    blobs: None,
                },
            )
            .await?,
        );

        let database_arc = Arc::new((*database).clone());
        let repos = Arc::clone(database_arc.repositories());
        let mut context = AdminApiContext::new(AdminApiContextInit {
            database: Arc::clone(&database_arc),
            repos: Arc::clone(&repos),
            jwt_secret: TEST_ADMIN_JWT_SECRET.to_owned(),
            auth_manager: auth_manager.clone(),
            jwks_manager: Arc::clone(&jwks),
            admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
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
            app_behavior: AppBehaviorConfig::default(),
        });
        let invited = Arc::new(Mutex::new(Vec::new()));
        context.approval_notifier = Some(Arc::new(RecordingNotifier {
            invited: Arc::clone(&invited),
        }));
        context.frontend_url = Some(FRONTEND_URL.to_owned());

        let operator_email = "team-operator@example.com".to_owned();
        let operator_id = seed_user(&repos, &operator_email, UserRole::SuperAdmin).await?;

        Ok(Self {
            resources,
            admin: AdminRoutes::routes(context),
            repos,
            jwks,
            operator_id,
            operator_email,
            invited,
        })
    }

    /// A super-admin token acting as the operator, as the console session is.
    async fn super_admin_token(&self) -> Result<String> {
        self.token(true).await
    }

    async fn token(&self, is_super_admin: bool) -> Result<String> {
        let request = CreateAdminTokenRequest {
            service_name: format!("console:{}", Uuid::new_v4()),
            service_description: Some("seeded by coach_seat_invitation_test".to_owned()),
            permissions: None,
            expires_in_days: Some(30),
            is_super_admin,
            tenant_id: None,
            operator_user_id: Some(self.operator_id),
        };
        let generated = self
            .repos
            .admin
            .create_token(&request, TEST_ADMIN_JWT_SECRET, &self.jwks)
            .await?;
        Ok(generated.jwt_token.expose_secret().to_owned())
    }

    async fn invite(&self, token: &str, email: &str) -> (u16, Value) {
        let response = AxumTestRequest::post("/admin/coach-seat-invitations")
            .header("Authorization", &format!("Bearer {token}"))
            .json(&json!({ "email": email }))
            .send(self.admin.clone())
            .await;
        let status = response.status();
        (status, response.json())
    }

    async fn list(&self, token: &str) -> (u16, Value) {
        let response = AxumTestRequest::get("/admin/coach-seat-invitations")
            .header("Authorization", &format!("Bearer {token}"))
            .send(self.admin.clone())
            .await;
        let status = response.status();
        (status, response.json())
    }

    async fn revoke(&self, token: &str, email: &str) -> (u16, Value) {
        let path = format!(
            "/admin/coach-seat-invitations/{}",
            urlencoding::encode(email)
        );
        let response = AxumTestRequest::delete(&path)
            .header("Authorization", &format!("Bearer {token}"))
            .send(self.admin.clone())
            .await;
        let status = response.status();
        (status, response.json())
    }

    fn mailed(&self) -> Vec<MailedInvitation> {
        self.invited
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// POST the real acceptance handler with `body` and return `(status, body)`.
    async fn accept_with(&self, body: Value) -> (u16, Value) {
        let response = AxumTestRequest::post("/api/auth/accept-invitation")
            .json(&body)
            .send(AuthRoutes::routes(self.resources.auth_routes_context()))
            .await;
        let status = response.status();
        (status, response.json())
    }

    async fn accept(&self, token: &str) -> (u16, Value) {
        self.accept_with(
            json!({ "token": token, "password": PASSWORD, "display_name": "Coach Phil" }),
        )
        .await
    }

    async fn user(&self, email: &str) -> Option<User> {
        self.repos.users.get_by_email(email).await.unwrap()
    }

    /// The onboarding status the invited coach's app reads after sign-in.
    async fn onboarding_status(&self, user: &User) -> Value {
        let jwt = common::generate_test_token(&self.resources, user).await;
        let response = AxumTestRequest::get("/api/me/onboarding-status")
            .header("Authorization", &format!("Bearer {jwt}"))
            .send(OnboardingRoutes::routes(Arc::clone(&self.resources)))
            .await;
        assert_eq!(response.status(), 200, "{}", response.body_text());
        response.json()
    }

    /// The recorded status of each onboarding step, by id.
    async fn step_status(&self, user_id: Uuid, step_id: &str) -> Option<String> {
        self.repos
            .user_onboarding
            .get_onboarding_steps(&user_id.to_string())
            .await
            .unwrap()
            .into_iter()
            .find(|step| step.step_id == step_id)
            .map(|step| step.status)
    }
}

async fn seed_user(repos: &Arc<RepositoryRegistry>, email: &str, role: UserRole) -> Result<Uuid> {
    let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST)?;
    let mut user = User::new(email.to_owned(), password_hash, Some("Seeded".to_owned()));
    user.user_status = UserStatus::Active;
    user.role = role;
    user.is_admin = role.is_admin_or_higher();
    let id = user.id;
    repos.users.create(&user).await?;
    Ok(id)
}

fn invitations(body: &Value) -> Vec<Value> {
    body["data"]["invitations"]
        .as_array()
        .cloned()
        .unwrap_or_default()
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
async fn redeeming_a_coach_seat_invitation_grants_the_seat_and_records_the_coach_role() -> Result<()>
{
    let harness = Harness::new().await?;
    let token = harness.super_admin_token().await?;
    let (status, body) = harness.invite(&token, "Coach.Phil@Example.com").await;
    assert_eq!(status, 200, "the invite must be issued: {body}");
    assert_eq!(body["data"]["outcome"], "invited", "{body}");

    let mailed = harness.mailed();
    assert_eq!(mailed.len(), 1, "one invitation is mailed: {mailed:?}");
    assert_eq!(mailed[0].email, "coach.phil@example.com");
    assert_eq!(mailed[0].kind, InvitationKind::CoachSeat);
    assert!(
        mailed[0].url.ends_with("&kind=coach_seat"),
        "the link names its kind for the page's wording: {}",
        mailed[0].url
    );

    let (status, body) = harness.accept(&mailed[0].token).await;
    assert_eq!(status, 201, "acceptance must create the account: {body}");
    assert_eq!(body["email"], "coach.phil@example.com", "{body}");
    assert_eq!(body["user_status"], "active", "{body}");

    let coach = harness
        .user("coach.phil@example.com")
        .await
        .expect("the invited address has an account");
    assert!(coach.manages_roster, "the invitation grants coach access");
    let grant = harness
        .repos
        .users
        .manages_roster_operator_grant(coach.id)
        .await?
        .expect("coach access is an operator grant");
    assert_eq!(
        grant.granted_by,
        Some(harness.operator_id),
        "the inviter is the operator of the grant"
    );
    assert_eq!(coach.approved_by, Some(harness.operator_id));
    assert!(
        harness.repos.users.coaches_others(coach.id).await?,
        "the coach role is recorded"
    );

    for (step, expected) in [
        ("profile_type", "complete"),
        ("coach_seat", "complete"),
        ("about_you", "not_applicable"),
        ("parq", "not_applicable"),
    ] {
        assert_eq!(
            harness.step_status(coach.id, step).await.as_deref(),
            Some(expected),
            "step {step}"
        );
    }

    let status = harness.onboarding_status(&coach).await;
    assert_eq!(status["coaches_others"], true, "{status}");
    assert_eq!(
        status["settled_by_invite"], true,
        "the bar shows the role and the seat as settled by the invitation: {status}"
    );
    Ok(())
}

#[tokio::test]
async fn a_plain_invitation_settles_neither_the_seat_nor_the_role() -> Result<()> {
    let harness = Harness::new().await?;
    let issued = match invitation::issue(
        &harness.repos,
        "athlete@example.com",
        Some(harness.operator_id),
        InvitationKind::Account,
    )
    .await?
    {
        InviteOutcome::Issued(issued) => issued,
        other => panic!("expected an invitation, got {other:?}"),
    };
    let (status, body) = harness.accept(&issued.token).await;
    assert_eq!(status, 201, "{body}");

    let athlete = harness
        .user("athlete@example.com")
        .await
        .expect("account created");
    assert!(!athlete.manages_roster, "a plain invitation grants no seat");
    assert!(!harness.repos.users.coaches_others(athlete.id).await?);
    assert_eq!(harness.step_status(athlete.id, "coach_seat").await, None);
    assert_eq!(harness.step_status(athlete.id, "profile_type").await, None);
    let status = harness.onboarding_status(&athlete).await;
    assert_eq!(status["settled_by_invite"], false, "{status}");
    Ok(())
}

#[tokio::test]
async fn the_seat_goes_to_the_invited_address_and_no_other() -> Result<()> {
    let harness = Harness::new().await?;
    let token = harness.super_admin_token().await?;
    let (status, body) = harness.invite(&token, "invited-coach@example.com").await;
    assert_eq!(status, 200, "{body}");
    let link = harness.mailed().remove(0);

    // The form carries no address: one sent along is not read, and the
    // account is created for the invitation's address all the same.
    let (status, body) = harness
        .accept_with(json!({
            "token": link.token,
            "password": PASSWORD,
            "email": "someone-else@example.com",
        }))
        .await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["email"], "invited-coach@example.com", "{body}");
    assert!(
        harness.user("someone-else@example.com").await.is_none(),
        "no account is created for another address"
    );
    assert!(
        harness
            .user("invited-coach@example.com")
            .await
            .expect("the invited address has the account")
            .manages_roster
    );

    // Someone else holding the spent link gets nothing either.
    let (status, body) = harness.accept(&link.token).await;
    assert_uniform_refusal(status, &body, "a redeemed coach-seat link");

    // Signing up another address the ordinary way holds no seat.
    let other = seed_user(&harness.repos, "someone-else@example.com", UserRole::User).await?;
    let other = harness
        .repos
        .users
        .get_global(other)
        .await?
        .expect("seeded");
    assert!(
        !other.manages_roster,
        "the seat never follows another address"
    );
    Ok(())
}

#[tokio::test]
async fn an_expired_coach_seat_invitation_is_refused() -> Result<()> {
    let harness = Harness::new().await?;
    let generated = generate_link_token();
    harness
        .repos
        .user_invitations
        .issue(
            "late-coach@example.com",
            Some(harness.operator_id),
            &generated.selector,
            &generated.verifier_hash,
            Utc::now() - Duration::minutes(1),
            InvitationKind::CoachSeat,
        )
        .await?;

    let (status, body) = harness.accept(&generated.token).await;
    assert_uniform_refusal(status, &body, "an expired coach-seat link");
    assert!(harness.user("late-coach@example.com").await.is_none());

    // The console still lists it, marked expired, so it can be sent again.
    let token = harness.super_admin_token().await?;
    let (status, body) = harness.list(&token).await;
    assert_eq!(status, 200, "{body}");
    let listed = invitations(&body);
    assert_eq!(listed.len(), 1, "{body}");
    assert_eq!(listed[0]["email"], "late-coach@example.com");
    assert_eq!(listed[0]["expired"], true, "{body}");
    Ok(())
}

#[tokio::test]
async fn a_coach_seat_invitation_revoked_from_the_console_is_refused() -> Result<()> {
    let harness = Harness::new().await?;
    let token = harness.super_admin_token().await?;
    let (status, body) = harness.invite(&token, "withdrawn@example.com").await;
    assert_eq!(status, 200, "{body}");
    let link = harness.mailed().remove(0);

    let (status, body) = harness.revoke(&token, "withdrawn@example.com").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["data"]["revoked"], true, "{body}");

    let (status, body) = harness.list(&token).await;
    assert_eq!(status, 200, "{body}");
    assert!(
        invitations(&body).is_empty(),
        "a revoked invite leaves the queue: {body}"
    );

    let (status, body) = harness.accept(&link.token).await;
    assert_uniform_refusal(status, &body, "a revoked coach-seat link");
    assert!(harness.user("withdrawn@example.com").await.is_none());

    let (status, body) = harness.revoke(&token, "withdrawn@example.com").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["data"]["revoked"], false,
        "nothing is left to revoke: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn an_address_with_an_account_is_offered_the_direct_grant_instead() -> Result<()> {
    let harness = Harness::new().await?;
    let token = harness.super_admin_token().await?;
    let existing = seed_user(&harness.repos, "already-here@example.com", UserRole::User).await?;

    let (status, body) = harness.invite(&token, "Already-Here@example.com").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["data"]["outcome"], "account_exists", "{body}");
    assert_eq!(body["data"]["user_id"], existing.to_string(), "{body}");
    assert_eq!(body["data"]["manages_roster"], false, "{body}");
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|m| m.contains("grant it coach access directly")),
        "the console is told to use the direct grant: {body}"
    );
    assert!(harness.mailed().is_empty(), "nothing is mailed");
    let (_, listed) = harness.list(&token).await;
    assert!(
        invitations(&listed).is_empty(),
        "no invitation is issued: {listed}"
    );
    Ok(())
}

#[tokio::test]
async fn the_console_lists_each_outstanding_coach_seat_invitation_with_its_expiry() -> Result<()> {
    let harness = Harness::new().await?;
    let token = harness.super_admin_token().await?;
    let (status, _) = harness.invite(&token, "first-coach@example.com").await;
    assert_eq!(status, 200);
    let (status, body) = harness.invite(&token, "first-coach@example.com").await;
    assert_eq!(status, 200, "sending again re-issues: {body}");
    // A plain invitation is not a coach seat, so the queue leaves it out.
    invitation::issue(
        &harness.repos,
        "plain-invitee@example.com",
        Some(harness.operator_id),
        InvitationKind::Account,
    )
    .await?;

    let (status, body) = harness.list(&token).await;
    assert_eq!(status, 200, "{body}");
    let listed = invitations(&body);
    assert_eq!(
        listed.len(),
        1,
        "one row per address, plain ones left out: {body}"
    );
    let row = &listed[0];
    assert_eq!(row["email"], "first-coach@example.com");
    assert_eq!(row["invited_by_email"], harness.operator_email.as_str());
    assert_eq!(row["expired"], false);
    let expires_at = row["expires_at"].as_str().expect("an expiry");
    let expires_at = chrono::DateTime::parse_from_rfc3339(expires_at)?;
    assert!(
        expires_at
            > Utc::now() + Duration::days(invitation::INVITATION_TTL_DAYS) - Duration::minutes(1),
        "the invitation TTL applies: {expires_at}"
    );

    // Sending again replaced the first link: only the second redeems.
    let mailed = harness.mailed();
    assert_eq!(mailed.len(), 2);
    let (selector, hash) = split_link_token(&mailed[0].token).expect("a token");
    assert!(
        harness
            .repos
            .user_invitations
            .redeem(&selector, &hash)
            .await
            .is_err(),
        "the earlier link stopped working"
    );
    let (status, body) = harness.accept(&mailed[1].token).await;
    assert_eq!(status, 201, "{body}");
    let (_, body) = harness.list(&token).await;
    assert!(
        invitations(&body).is_empty(),
        "a redeemed invite leaves the queue: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn revoking_a_coach_seat_leaves_a_plain_invitation_alone() -> Result<()> {
    let harness = Harness::new().await?;
    let token = harness.super_admin_token().await?;
    let issued = match invitation::issue(
        &harness.repos,
        "plain@example.com",
        None,
        InvitationKind::Account,
    )
    .await?
    {
        InviteOutcome::Issued(issued) => issued,
        other => panic!("expected an invitation, got {other:?}"),
    };
    let (status, body) = harness.revoke(&token, "plain@example.com").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["data"]["revoked"], false, "{body}");
    let (status, body) = harness.accept(&issued.token).await;
    assert_eq!(status, 201, "the plain invitation still works: {body}");
    Ok(())
}

#[tokio::test]
async fn only_a_super_admin_reaches_coach_seat_invitations() -> Result<()> {
    let harness = Harness::new().await?;
    let token = harness.token(false).await?;
    let (status, body) = harness.invite(&token, "nope@example.com").await;
    assert_eq!(status, 403, "{body}");
    let (status, body) = harness.list(&token).await;
    assert_eq!(status, 403, "{body}");
    let (status, body) = harness.revoke(&token, "nope@example.com").await;
    assert_eq!(status, 403, "{body}");
    assert!(harness.mailed().is_empty());
    Ok(())
}
