// ABOUTME: E2e coverage for the per-email pre-approval allow-list (pierre-cli user allow)
// ABOUTME: Register lands Active with admin attribution; disallow and pending-login paths covered
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The per-email allow-list closes half of registre#43: an admin records an
//! allow for an address *before* the person has an account (`pierre-cli user
//! allow --email X`), and the registration approval decision consumes it —
//! the account lands `Active` with `approved_by` attributed to the admin,
//! skipping the pending queue.
//!
//! These tests drive the real `/api/auth/register` handler and the apps'
//! sign-in (hosted login page, code redeemed at `/oauth/token`) over one
//! in-memory database, with global
//! auto-approval off and no `AUTO_APPROVE_DOMAINS` — so any `Active` outcome
//! here is the allow-list's doing, and a returns-empty stub of the repository
//! would fail the attribution and status assertions.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;
use pierre_test_support::account::delete_account;

use helpers::axum_test::AxumTestRequest;
use helpers::first_party_sign_in::SignIn;
use helpers::notify_capture::{capture_notify, named, only};
use pierre_auth::dto::auth::LoginRequest;
use pierre_config::environment::{
    AppBehaviorConfig, BackupConfig, DatabaseConfig, DatabaseUrl, Environment, SecurityConfig,
    SecurityHeadersConfig, ServerConfig,
};
use pierre_core::errors::ErrorCode;
use pierre_core::models::{User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_mcp_server::mcp::resources::{ServerContext, ServerContextOptions};
use pierre_routes_auth::AuthRoutes;
use pierre_services::admin_ops::AccountCaller;
use pierre_services::auth::{FederatedIdentity, SignupSource};
use pierre_services::pre_approval::{self, AllowOutcome};
use secrecy::SecretString;
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
            common::offline_config(config),
            cache,
            ServerContextOptions {
                rsa_key_size_bits: Some(2048),
                jwks_manager: Some(common::get_shared_test_jwks()),
                llm_provider: None,
                chat_provider: None,
                extra_tools: Vec::new(),
                billing_provider: None,
                turn_runner: None,
                archive_imports: None,
                live_records: None,
                blobs: None,
                lab_reader: None,
                pdf_text_bin: None,
                contremaitre: None,
                #[cfg(feature = "client-messaging")]
                channel_adapters: common::offline_adapters(),
            },
        )
        .await
        .unwrap(),
    );

    Ok(resources)
}

/// Register a real account to act as the allow-listing admin:
/// `users.approved_by` references `users.id`, so attribution needs a row.
async fn register_admin(resources: &Arc<ServerContext>, email: &str) -> Uuid {
    let (status, body) = register(resources, email, "adminPassword123").await;
    assert_eq!(status, 201, "admin registration must succeed: {body}");
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
    let admin = register_admin(&resources, "allow-admin-1@example.com").await;
    let email = "alpha-cohort-1@example.com";

    let added = resources
        .common
        .repos
        .pre_approved_emails
        .allow(email, Some(admin), Some("alpha cohort"))
        .await
        .expect("recording the allow must succeed");
    assert!(added, "first allow of a new address must report insertion");

    let (events, guard) = capture_notify();
    let (status, body) = register(&resources, email, "securePassword123").await;
    drop(guard);
    assert_eq!(status, 201, "registration must succeed: {body}");
    assert_eq!(
        body["user_status"].as_str(),
        Some("active"),
        "a pre-approved email must land active, got: {body}"
    );
    assert!(
        named(&events, "user.pending_approval").is_empty(),
        "an account that lands active waits on no admin"
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
        Some(admin),
        "approved_by must attribute the allow-listing admin"
    );
    assert!(
        user.approved_at.is_some(),
        "approved_at must be stamped on auto-approval"
    );
}

#[tokio::test]
async fn an_allow_outliving_its_admin_still_registers_active_unattributed() {
    let resources = setup().await.expect("server context setup failed");
    let repos = &resources.common.repos;
    let admin = register_admin(&resources, "departed-allow-admin@example.com").await;
    let email = "outlived-allow@example.com";
    repos
        .pre_approved_emails
        .allow(email, Some(admin), Some("alpha cohort"))
        .await
        .expect("recording the allow must succeed");

    delete_account(repos, admin, None)
        .await
        .expect("an admin whose only trace is an allow can be deleted");

    let (status, body) = register(&resources, email, "securePassword123").await;
    assert_eq!(
        status, 201,
        "the allow must still let the address in: {body}"
    );
    assert_eq!(body["user_status"].as_str(), Some("active"), "{body}");
    let user = repos
        .users
        .get_by_email(email)
        .await
        .expect("user lookup must succeed")
        .expect("registered user must exist");
    assert_eq!(
        user.approved_by, None,
        "a deleted admin cannot be the account's approver"
    );
    assert!(
        user.approved_at.is_some(),
        "the approval itself still stands"
    );
}

#[tokio::test]
async fn unlisted_email_registers_pending() {
    let resources = setup().await.expect("server context setup failed");

    let (events, guard) = capture_notify();
    let (status, body) = register(&resources, "stranger@example.com", "securePassword123").await;
    drop(guard);
    assert_eq!(status, 201, "registration must succeed: {body}");
    assert_eq!(
        body["user_status"].as_str(),
        Some("pending"),
        "an unlisted email must land in the pending queue, got: {body}"
    );

    // The admins hear about it (carnet#1056): one ping naming the account
    // and the personal workspace it was given.
    let user_id = body["user_id"].as_str().expect("user_id");
    let ping = only(&events, "user.pending_approval");
    assert_eq!(ping.field("user_id"), user_id);
    let tenants = resources
        .common
        .repos
        .tenants
        .list_for_user(Uuid::parse_str(user_id).expect("user_id must be a UUID"))
        .await
        .expect("tenants of the new account");
    assert_eq!(ping.field("tenant_id"), tenants[0].id.to_string());
}

/// An address an applicant types reaches the approvals room as the subject of
/// the ping, so one Slack would render as a room-wide mention is refused at
/// the door and pings nobody (carnet#1056).
#[tokio::test]
async fn an_address_carrying_slack_markup_is_refused_and_pings_nobody() {
    let resources = setup().await.expect("server context setup failed");

    let (events, guard) = capture_notify();
    let (status, body) = register(&resources, "<!channel>@example.com", "securePassword123").await;
    drop(guard);
    assert_eq!(status, 400, "the address must be refused: {body}");
    assert!(named(&events, "user.pending_approval").is_empty());
    assert!(named(&events, "user.signed_up").is_empty());
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
    let admin = Uuid::new_v4();

    assert!(repos
        .pre_approved_emails
        .allow("dupe@example.com", Some(admin), Some("first"))
        .await
        .expect("first allow must succeed"));
    assert!(
        !repos
            .pre_approved_emails
            .allow("dupe@example.com", Some(admin), Some("second"))
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
    assert_eq!(entries[0].allowed_by, Some(admin));
}

#[tokio::test]
async fn pending_user_promoted_on_next_login_with_attribution() {
    let resources = setup().await.expect("server context setup failed");
    let repos = &resources.common.repos;
    let admin = register_admin(&resources, "late-allow-admin@example.com").await;
    let email = "late-allow@example.com";
    let password = "securePassword123";

    // Registered before any allow existed → pending queue.
    let (status, body) = register(&resources, email, password).await;
    assert_eq!(status, 201, "registration must succeed: {body}");
    assert_eq!(body["user_status"].as_str(), Some("pending"));

    // The admin allows the exact address afterwards.
    repos
        .pre_approved_emails
        .allow(email, Some(admin), None)
        .await
        .expect("recording the allow must succeed");

    // Next login retroactively promotes the pending account.
    let login_resp = SignIn::new(email, password).run(&resources).await.token();
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
        Some(admin),
        "retroactive promotion must attribute the allow-listing admin"
    );
}

/// An admin is approved by an admin, never by a sign-up rule (carnet#1589).
/// A pending account that holds the admin role (set back to pending, or
/// given the role by a write that did not look at its status) stays pending
/// at its next login though a standing allow names its address, where a
/// sign-up in the same position is let in
/// (`pending_user_promoted_on_next_login_with_attribution`).
#[tokio::test]
async fn a_pending_admin_stays_pending_on_login_whatever_allows_its_address() {
    let resources = setup().await.expect("server context setup failed");
    let repos = &resources.common.repos;
    let allowed_by = register_admin(&resources, "allowing-admin@example.com").await;
    let email = "pending-admin@example.com";
    let password = "securePassword123";

    let (status, body) = register(&resources, email, password).await;
    assert_eq!(status, 201, "registration must succeed: {body}");
    assert_eq!(body["user_status"].as_str(), Some("pending"));
    // The role, written past the promotion (which refuses a pending account).
    let mut account = repos
        .users
        .get_by_email(email)
        .await
        .unwrap()
        .expect("the account");
    account.role = UserRole::Admin;
    repos.users.update(&account).await.unwrap();
    repos
        .pre_approved_emails
        .allow(email, Some(allowed_by), None)
        .await
        .expect("recording the allow must succeed");

    let login = SignIn::new(email, password).run(&resources).await.token();
    assert_eq!(login.status(), 200, "a pending account still signs in");
    // Only the account is printed on a failure: the body beside it is the
    // session's tokens.
    let signed_in: Value = login.json::<Value>()["user"].clone();
    assert_eq!(
        signed_in["user_status"].as_str(),
        Some("pending"),
        "the login activated an admin nobody approved: {signed_in}"
    );

    let stored = repos
        .users
        .get_by_email(email)
        .await
        .unwrap()
        .expect("the account");
    assert!(
        stored.role.is_admin(),
        "the fixture held the role throughout"
    );
    assert_eq!(stored.user_status, UserStatus::Pending);
    assert_eq!(
        stored.approved_by, None,
        "and nobody is recorded as approving it"
    );
}

/// The allow-list matches an address, not a spelling of it: a registration
/// differing from the allow only in casing and surrounding whitespace is the
/// allowed person, and lands active.
#[tokio::test]
async fn a_registration_in_another_casing_matches_the_allow() {
    let resources = setup().await.expect("server context setup failed");
    let admin = register_admin(&resources, "casing-admin-1@example.com").await;

    resources
        .common
        .repos
        .pre_approved_emails
        .allow("cohort.two@example.com", Some(admin), None)
        .await
        .expect("recording the allow must succeed");

    let (status, body) =
        register(&resources, "  Cohort.Two@EXAMPLE.com ", "securePassword123").await;
    assert_eq!(status, 201, "registration must succeed: {body}");
    assert_eq!(
        body["user_status"].as_str(),
        Some("active"),
        "casing and whitespace must not defeat the allow, got: {body}"
    );
    let user = resources
        .common
        .repos
        .users
        .get_by_email("cohort.two@example.com")
        .await
        .expect("user lookup must succeed")
        .expect("registered user must exist");
    assert_eq!(user.email, "cohort.two@example.com");
    assert_eq!(user.approved_by, Some(admin));
}

/// An allow recorded after the person registered in another casing finds
/// their pending account and approves it, instead of recording a standing
/// allow nobody will ever register against again.
#[tokio::test]
async fn an_allow_approves_a_pending_account_registered_in_another_casing() {
    let resources = setup().await.expect("server context setup failed");
    let repos = &resources.common.repos;
    let admin = register_admin(&resources, "casing-admin-2@example.com").await;

    let (status, body) = register(&resources, "Late.Cohort@Example.COM", "securePassword123").await;
    assert_eq!(status, 201, "registration must succeed: {body}");
    assert_eq!(body["user_status"].as_str(), Some("pending"));

    let host = AccountCaller::host_cli(Some(admin));
    let result = pre_approval::allow(repos, &host, "late.cohort@example.com", None)
        .await
        .expect("the allow must succeed");
    assert_eq!(result.outcome, AllowOutcome::PendingApproved);
    assert_eq!(
        result.approved_user.map(|u| u.email).as_deref(),
        Some("late.cohort@example.com")
    );

    let user = repos
        .users
        .get_by_email("LATE.COHORT@example.com")
        .await
        .expect("user lookup must succeed")
        .expect("user must exist");
    assert_eq!(user.user_status, UserStatus::Active);
    assert_eq!(user.approved_by, Some(admin));
    assert!(
        repos
            .pre_approved_emails
            .get("late.cohort@example.com")
            .await
            .expect("lookup must succeed")
            .is_none(),
        "the pending account was approved, not a standing allow recorded"
    );
}

/// A pending account registered at `email`, as the store holds it.
async fn pending_account(resources: &Arc<ServerContext>, email: &str, password: &str) -> User {
    let (status, body) = register(resources, email, password).await;
    assert_eq!(status, 201, "registration must succeed: {body}");
    assert_eq!(body["user_status"].as_str(), Some("pending"));
    resources
        .common
        .repos
        .users
        .get_by_email(email)
        .await
        .expect("user lookup must succeed")
        .expect("user must exist")
}

/// A suspension that lands while a login is being answered stands
/// (carnet#1589). The login reads the account pending, takes its time over
/// the password, and only then activates the account a standing allow lets
/// in; an admin who rejects the sign-up in between has decided it. The
/// activation is written only while the account is still pending, so the
/// account stays suspended and the login is refused as any suspended
/// account's is. Checked on the earlier read and then written, the login
/// set the account active again and signed its holder in.
#[tokio::test]
async fn a_login_never_reactivates_an_account_suspended_while_it_was_answered() {
    let resources = setup().await.expect("server context setup failed");
    let repos = &resources.common.repos;
    let (email, password) = ("suspended-mid-login@example.com", "securePassword123");
    let user = pending_account(&resources, email, password).await;
    repos
        .pre_approved_emails
        .allow(email, None, None)
        .await
        .expect("recording the allow must succeed");
    let service = resources.auth_routes_context().account_service();

    // The login is polled first, so its read of the account is the first
    // statement; the suspension is done long before the password is checked.
    let (login, suspension) = tokio::join!(
        service.login(LoginRequest {
            email: email.to_owned(),
            password: SecretString::from(password),
            timezone: None,
        }),
        repos
            .users
            .update_status(user.id, UserStatus::Suspended, None),
    );

    suspension.expect("the suspension is written");
    let stored = repos.users.get_global(user.id).await.unwrap().unwrap();
    assert_eq!(
        stored.user_status,
        UserStatus::Suspended,
        "the suspension stands"
    );
    let refused = login.expect_err("a suspended account is not signed in");
    assert_eq!(refused.code, ErrorCode::AccountSuspended, "{refused}");
}

/// The same for a federated sign-in (carnet#1589): it finds the pending
/// account by its Firebase UID, and a suspension that lands before the
/// allow's activation is written stands. The sign-in is refused, and the
/// account is not handed a session.
#[tokio::test]
async fn a_federated_sign_in_never_reactivates_an_account_suspended_meanwhile() {
    let resources = setup().await.expect("server context setup failed");
    let repos = &resources.common.repos;
    let email = "suspended-mid-google@example.com";
    let mut user = User::new(email.to_owned(), "not-a-login-path".to_owned(), None);
    user.user_status = UserStatus::Pending;
    user.firebase_uid = Some("fb-suspended-mid-sign-in".to_owned());
    user.auth_provider = "google.com".to_owned();
    repos.users.create(&user).await.unwrap();
    repos
        .pre_approved_emails
        .allow(email, None, None)
        .await
        .expect("recording the allow must succeed");
    let service = resources.auth_routes_context().account_service();

    // The sign-in is polled first, so its read of the account by UID is the
    // first statement.
    let (sign_in, suspension) = tokio::join!(
        service.login_with_federated_identity(FederatedIdentity {
            firebase_uid: Some("fb-suspended-mid-sign-in"),
            google_subject: None,
            email: email.to_owned(),
            email_verified: true,
            display_name: None,
            provider: "google.com",
            signup_source: SignupSource::Firebase,
            locale: None,
        }),
        repos
            .users
            .update_status(user.id, UserStatus::Suspended, None),
    );

    suspension.expect("the suspension is written");
    let stored = repos.users.get_global(user.id).await.unwrap().unwrap();
    assert_eq!(
        stored.user_status,
        UserStatus::Suspended,
        "the suspension stands"
    );
    // Signed in before the suspension landed, the account was active then.
    match sign_in {
        Ok(signed_in) => assert_eq!(signed_in.user.user_status, "active"),
        Err(refused) => assert_eq!(refused.code, ErrorCode::AccountSuspended, "{refused}"),
    }
}

/// An allow recorded while an admin rejects the same sign-up never
/// overrides the rejection (carnet#1589): whichever order the two land in,
/// the account ends suspended. An allow that finds the sign-up already
/// decided says so, approves nobody and provisions nothing. Checked and
/// then written, it set the suspended account active.
#[tokio::test]
async fn an_allow_never_reactivates_an_account_suspended_while_it_was_recorded() {
    let resources = setup().await.expect("server context setup failed");
    let repos = &resources.common.repos;
    let email = "suspended-mid-allow@example.com";
    let user = pending_account(&resources, email, "securePassword123").await;

    let host = AccountCaller::host_cli(None);
    let (allowed, suspension) = tokio::join!(
        pre_approval::allow(repos, &host, email, None),
        repos
            .users
            .update_status(user.id, UserStatus::Suspended, None),
    );

    suspension.expect("the suspension is written");
    let allowed = allowed.expect("the allow answers");
    let stored = repos.users.get_global(user.id).await.unwrap().unwrap();
    assert_eq!(
        stored.user_status,
        UserStatus::Suspended,
        "the suspension stands"
    );
    // The allow either landed first, and was then suspended like any active
    // account, or found the account decided.
    if allowed.outcome != AllowOutcome::PendingApproved {
        assert_eq!(allowed.outcome, AllowOutcome::SuspendedUnchanged);
        assert!(allowed.approved_user.is_none(), "nobody to announce");
        let tokens = repos.user_mcp_tokens.list_tokens(user.id).await.unwrap();
        assert!(tokens.is_empty(), "nothing is provisioned");
    }
}

/// What an admin does to an account while its first federated sign-in is
/// being linked.
#[derive(Clone, Copy, Debug)]
enum MeanwhileWrite {
    Suspends,
    Demotes,
    Promotes,
}

/// How many accounts each admin write is raced on: the order the link and
/// the write reach the store in is the runtime's, so one round proves little.
const LINK_RACE_ROUNDS: usize = 4;

/// A first federated sign-in that finds an account by its email links the
/// identity to it (carnet#1589). The link read the account, took its time
/// over the account's earlier credentials, and then wrote the whole row
/// back from what it had read, role, status and activity included: a
/// suspension, a demotion or a promotion that landed in between was undone
/// by a sign-in. The link writes the identity it proved and nothing else,
/// so whichever order the two land in, the standing is the admin's.
#[tokio::test]
async fn a_first_federated_link_never_writes_back_the_standing_an_admin_changed_meanwhile() {
    let resources = setup().await.expect("server context setup failed");
    let repos = &resources.common.repos;
    // A second admin in good standing, so a demotion is never the last one's.
    let mut standing = User::new("standing@example.com".to_owned(), "hash".to_owned(), None);
    standing.user_status = UserStatus::Active;
    standing.role = UserRole::Admin;
    repos.users.create(&standing).await.unwrap();
    let service = resources.auth_routes_context().account_service();
    let mut undone = Vec::new();

    for write in [
        MeanwhileWrite::Suspends,
        MeanwhileWrite::Demotes,
        MeanwhileWrite::Promotes,
    ] {
        for round in 0..LINK_RACE_ROUNDS {
            let email = format!("{write:?}-mid-link-{round}@example.com").to_lowercase();
            // Registered with a password and never verified: the link has
            // the account's earlier credentials to retire before it writes.
            let mut user = User::new(email.clone(), "not-a-login-path".to_owned(), None);
            user.user_status = UserStatus::Active;
            user.role = match write {
                MeanwhileWrite::Demotes => UserRole::Admin,
                MeanwhileWrite::Suspends | MeanwhileWrite::Promotes => UserRole::User,
            };
            repos.users.create(&user).await.unwrap();

            // The sign-in is polled first, and its identity carries no
            // provider id to look an account up by, so its read of the
            // account by email is the first statement; the admin's write
            // lands while it works.
            let (sign_in, written) = tokio::join!(
                service.login_with_federated_identity(FederatedIdentity {
                    firebase_uid: None,
                    google_subject: None,
                    email: email.clone(),
                    email_verified: true,
                    display_name: None,
                    provider: "google.com",
                    signup_source: SignupSource::Google,
                    locale: None,
                }),
                async {
                    match write {
                        MeanwhileWrite::Suspends => {
                            repos
                                .users
                                .update_status(user.id, UserStatus::Suspended, None)
                                .await
                        }
                        MeanwhileWrite::Demotes => {
                            repos.users.set_admin_status(user.id, false).await
                        }
                        MeanwhileWrite::Promotes => {
                            repos.users.set_admin_status(user.id, true).await
                        }
                    }
                },
            );

            written.expect("the admin's write lands");
            let stored = repos.users.get_global(user.id).await.unwrap().unwrap();
            let standing = (stored.user_status, stored.role, stored.is_active);
            let expected = match write {
                MeanwhileWrite::Suspends => (UserStatus::Suspended, UserRole::User, true),
                MeanwhileWrite::Demotes => (UserStatus::Active, UserRole::User, true),
                MeanwhileWrite::Promotes => (UserStatus::Active, UserRole::Admin, true),
            };
            if standing != expected {
                undone.push(format!(
                    "{write:?}, round {round}: {standing:?}, not {expected:?}"
                ));
            }
            match sign_in {
                Ok(_) => assert_eq!(
                    stored.auth_provider, "google.com",
                    "{write:?}, round {round}: the sign-in that was answered linked the identity"
                ),
                // Suspended before the link was answered.
                Err(refused) => {
                    assert_eq!(refused.code, ErrorCode::AccountSuspended, "{refused}");
                }
            }
        }
    }
    // Every case is raced before any is judged, so a failure names each
    // kind of write a link undid.
    assert_eq!(
        undone,
        Vec::<String>::new(),
        "the admin's write stands, whichever order the two land in"
    );
}
