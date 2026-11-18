// ABOUTME: Bootstrap seeder role test — the bootstrap account lands as an admin, demo users as plain users
// ABOUTME: Guards the dev device-login approval path, which gates on the account's admin role
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use pierre_core::admin::jwt::JwtSigner;
use pierre_core::admin::models::CreateAdminTokenRequest;
use pierre_core::errors::AppResult;
use pierre_core::models::{User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_database::seed_models::SEED_LOCALE;
use pierre_database::RepositoryRegistry;
use pierre_seeders::bootstrap::{self, SeedArgs};
use serde_json::Value;

/// The demo password these tests hand the run: the demo and test accounts
/// are seeded only when it is given one.
const DEMO_PASSWORD: &str = "TestDemoPass123!";

/// The demo and test accounts the run seeds when it is given a demo password.
const DEMO_ACCOUNTS: [&str; 4] = [
    "alice@demo.pierre.dev",
    "bob@demo.pierre.dev",
    "phil_test@dravr.ai",
    "jf_test@dravr.ai",
];

/// A signer that only has to hand the repository a string to hash: these
/// tests never validate the token, they read its row.
struct Unsigned;

impl JwtSigner for Unsigned {
    fn sign_token(&self, claims: &Value) -> AppResult<String> {
        Ok(format!("unsigned.{claims}"))
    }
}

/// The bootstrap seeder must create the `ADMIN_EMAIL` account with the `admin` role. The
/// browser device-login approval page, `cookie_admin_middleware` and the admin listing all
/// gate on the role column — an account without it cannot approve a CLI sign-in and is
/// missing from the roster. Demo users stay non-privileged.
#[tokio::test]
async fn bootstrap_admin_is_an_admin_demo_users_are_plain() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();

    seed(repos).await.unwrap();

    let admin = repos
        .users
        .get_by_email("admin@dravr.ai")
        .await
        .unwrap()
        .expect("admin seeded");
    assert_eq!(
        admin.role,
        UserRole::Admin,
        "the seeded admin must hold the admin role so it can approve device logins"
    );

    let alice = repos
        .users
        .get_by_email("alice@demo.pierre.dev")
        .await
        .unwrap()
        .expect("demo user seeded");
    assert_eq!(
        alice.role,
        UserRole::User,
        "demo users must stay non-privileged"
    );
}

/// A seeding run in a fresh database, as `pierre-cli seed bootstrap` does it
/// when it is given a demo password.
async fn seed(repos: &RepositoryRegistry) -> AppResult<()> {
    seed_naming(repos, "admin@dravr.ai").await
}

/// A seeding run whose `ADMIN_EMAIL` is `admin_email`, given a demo password.
async fn seed_naming(repos: &RepositoryRegistry, admin_email: &str) -> AppResult<()> {
    seed_with(repos, admin_email, Some(DEMO_PASSWORD)).await
}

/// A seeding run whose `ADMIN_EMAIL` is `admin_email`, handed `demo_password`.
async fn seed_with(
    repos: &RepositoryRegistry,
    admin_email: &str,
    demo_password: Option<&str>,
) -> AppResult<()> {
    bootstrap::run(
        SeedArgs {
            admin_email: admin_email.to_owned(),
            admin_password: "AdminPass123!".to_owned(),
            demo_password: demo_password.map(str::to_owned),
        },
        repos,
    )
    .await
}

/// A re-run brings an existing account to its seeded role through the
/// repository's own write, not by SQL: the bootstrap admin someone demoted is
/// an admin again, and a demo account someone promoted goes back to a plain
/// user and loses the device login it approved as an admin (carnet#1574).
/// The upsert once asserted the role on conflict, with no guard and nothing
/// revoked.
#[tokio::test]
async fn a_rerun_restores_the_seeded_roles_and_withdraws_a_demoted_admins_credentials() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();
    seed(repos).await.unwrap();
    let by_email = |email: &'static str| async move {
        repos
            .users
            .get_by_email(email)
            .await
            .unwrap()
            .expect("seeded")
    };
    let alice = by_email("alice@demo.pierre.dev").await;
    let admin = by_email("admin@dravr.ai").await;
    repos.users.set_admin_status(alice.id, true).await.unwrap();
    repos.users.set_admin_status(admin.id, false).await.unwrap();
    let token = repos
        .admin
        .create_token(
            &CreateAdminTokenRequest {
                service_name: "device-cli:alice".to_owned(),
                service_description: None,
                permissions: None,
                expires_in_days: None,
                is_super_admin: true,
                tenant_id: None,
                operator_user_id: Some(alice.id),
            },
            "seeder-role-test-secret",
            &Unsigned,
        )
        .await
        .unwrap();

    seed(repos).await.unwrap();

    assert_eq!(
        by_email("admin@dravr.ai").await.role,
        UserRole::Admin,
        "the re-run makes the demoted bootstrap admin an admin again"
    );
    assert_eq!(
        by_email("alice@demo.pierre.dev").await.role,
        UserRole::User,
        "the re-run takes the promoted demo account back to a plain user"
    );
    let live = repos.admin.list_tokens(false).await.unwrap();
    assert!(
        live.iter().all(|row| row.id != token.token_id),
        "the device login the demoted demo account approved is deactivated"
    );
}

/// The re-run never takes the last active admin out of the role: the
/// repository refuses it, the run fails saying so, and the account keeps the
/// role. The SQL upsert demoted it without looking.
#[tokio::test]
async fn a_rerun_never_demotes_the_last_active_admin() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();
    seed(repos).await.unwrap();
    let alice = repos
        .users
        .get_by_email("alice@demo.pierre.dev")
        .await
        .unwrap()
        .expect("seeded");
    let admin = repos
        .users
        .get_by_email("admin@dravr.ai")
        .await
        .unwrap()
        .expect("seeded");
    repos.users.set_admin_status(alice.id, true).await.unwrap();
    repos
        .users
        .update_status(admin.id, UserStatus::Suspended, None)
        .await
        .unwrap();

    let refused = seed(repos).await.expect_err("the run is refused");
    assert!(
        refused.to_string().contains("last active admin"),
        "{refused}"
    );
    let alice = repos.users.get_global(alice.id).await.unwrap().unwrap();
    assert_eq!(
        alice.role,
        UserRole::Admin,
        "the one admin in standing stays"
    );
}

/// The seeder never makes an account that is not active an admin
/// (carnet#1589). `ADMIN_EMAIL` can name an account that signed up and
/// still waits for approval, or one an admin suspended: an admin is an
/// approved account first, the rule every other promotion takes, so the run
/// writes nothing on that account. Its role and status stay, and so do its
/// password, its display name and its locale: the run once replaced those
/// three before it asked the rule, which locked the holder out and gave
/// whoever holds `ADMIN_PASSWORD` the sign-in of an account the run said it
/// had left alone. It does not fail either: the demo users are seeded all
/// the same.
///
/// The rule is asked of the status, whatever role the account holds: an
/// account that already holds the admin role and is pending or suspended
/// keeps its own password too, and is not handed the bootstrap admin's.
#[tokio::test]
async fn a_run_writes_nothing_on_a_pending_or_suspended_account() {
    for (role, status) in [
        (UserRole::User, UserStatus::Pending),
        (UserRole::User, UserStatus::Suspended),
        (UserRole::Admin, UserStatus::Pending),
        (UserRole::Admin, UserStatus::Suspended),
    ] {
        let case = format!("{role:?}, {status}");
        let database = common::create_test_database().await.unwrap();
        let repos = database.repositories();
        let mut waiting = User::new(
            "waiting@dravr.ai".to_owned(),
            "not-a-login-path".to_owned(),
            Some("Waiting Holder".to_owned()),
        );
        waiting.role = role;
        waiting.user_status = status;
        repos.users.create(&waiting).await.unwrap();
        let created = repos.users.get_global(waiting.id).await.unwrap().unwrap();
        assert_eq!(created.role, role, "{case}: stored with its role");
        assert_ne!(created.locale, SEED_LOCALE, "the seed's locale would show");

        seed_naming(repos, "waiting@dravr.ai")
            .await
            .unwrap_or_else(|e| panic!("{case}: the run goes on: {e}"));

        let stored = repos.users.get_global(waiting.id).await.unwrap().unwrap();
        assert_eq!(
            stored.role, role,
            "{case}: an account that is not active is not made an admin, and keeps its role"
        );
        assert_eq!(stored.user_status, status, "the run decides no sign-up");
        assert_eq!(
            stored.password_hash, "not-a-login-path",
            "{case}: the account keeps its own password"
        );
        assert_eq!(
            stored.display_name.as_deref(),
            Some("Waiting Holder"),
            "{case}: and its own name"
        );
        assert_eq!(stored.locale, created.locale, "{case}: and its locale");
        let alice = repos.users.get_by_email("alice@demo.pierre.dev").await;
        assert!(
            alice.unwrap().is_some(),
            "{case}: the rest of the run is seeded"
        );
    }
}

/// The control of the rule above: an active account `ADMIN_EMAIL` names is
/// the bootstrap admin. The run makes it one and asserts its credentials, as
/// it does on every run.
#[tokio::test]
async fn a_run_makes_an_active_account_the_bootstrap_admin_with_its_credentials() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();
    let standing = User::new(
        "standing@dravr.ai".to_owned(),
        "not-a-login-path".to_owned(),
        Some("Standing Holder".to_owned()),
    );
    assert_eq!(standing.user_status, UserStatus::Active);
    repos.users.create(&standing).await.unwrap();

    seed_naming(repos, "standing@dravr.ai").await.unwrap();

    let stored = repos.users.get_global(standing.id).await.unwrap().unwrap();
    assert_eq!(stored.role, UserRole::Admin);
    assert_ne!(
        stored.password_hash, "not-a-login-path",
        "the run asserts the bootstrap admin's password"
    );
    assert_eq!(stored.display_name.as_deref(), Some("Admin"));
    assert_eq!(stored.locale, SEED_LOCALE);
}

/// The source holds no password for the demo and test accounts: the
/// repository is public (carnet#1597). A run handed none seeds the admin as
/// it always does and creates none of them on an empty database.
#[tokio::test]
async fn a_run_with_no_demo_password_creates_no_demo_account() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();

    seed_with(repos, "admin@dravr.ai", None).await.unwrap();

    let admin = repos
        .users
        .get_by_email("admin@dravr.ai")
        .await
        .unwrap()
        .expect("the admin path is unchanged");
    assert_eq!(
        admin.role,
        UserRole::Admin,
        "a run with no demo password still seeds the bootstrap admin"
    );
    for email in DEMO_ACCOUNTS {
        assert!(
            repos.users.get_by_email(email).await.unwrap().is_none(),
            "{email} is not created without a demo password"
        );
    }
}

/// A run handed a demo password seeds all four demo and test accounts under
/// that password, the only one they get: none keeps a value of its own.
#[tokio::test]
async fn a_run_with_a_demo_password_seeds_every_demo_account_under_it() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();

    seed(repos).await.unwrap();

    for email in DEMO_ACCOUNTS {
        let user = repos
            .users
            .get_by_email(email)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{email} is created with a demo password"));
        assert!(
            bcrypt::verify(DEMO_PASSWORD, &user.password_hash).unwrap(),
            "{email} signs in with the demo password the run was given"
        );
    }
}

/// An empty demo password is no password: an unset secret mounted as an
/// empty value must not seed the accounts with an empty password.
#[tokio::test]
async fn an_empty_demo_password_creates_no_demo_account() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();

    seed_with(repos, "admin@dravr.ai", Some("")).await.unwrap();

    for email in DEMO_ACCOUNTS {
        assert!(
            repos.users.get_by_email(email).await.unwrap().is_none(),
            "{email} is not created with an empty demo password"
        );
    }
}

/// On the deployed environment the demo and test accounts are managed out of
/// band, and the seed job runs on every deploy with no demo password. That
/// run must leave every such account as it is: its password hash, its role
/// and its status, even a role and a status a run given the password would
/// bring back to the seed's.
#[tokio::test]
async fn a_run_with_no_demo_password_leaves_existing_demo_accounts_untouched() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();
    seed(repos).await.unwrap();
    let alice = repos
        .users
        .get_by_email("alice@demo.pierre.dev")
        .await
        .unwrap()
        .expect("seeded");
    repos.users.set_admin_status(alice.id, true).await.unwrap();
    repos
        .users
        .update_status(alice.id, UserStatus::Suspended, None)
        .await
        .unwrap();
    let mut before = Vec::new();
    for email in DEMO_ACCOUNTS {
        let user = repos
            .users
            .get_by_email(email)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{email} seeded"));
        before.push(user);
    }

    seed_with(repos, "admin@dravr.ai", None).await.unwrap();

    for held in before {
        let stored = repos.users.get_global(held.id).await.unwrap().unwrap();
        assert_eq!(
            stored.password_hash, held.password_hash,
            "{}: the run takes no password from the source",
            held.email
        );
        assert_eq!(stored.role, held.role, "{}: keeps its role", held.email);
        assert_eq!(
            stored.user_status, held.user_status,
            "{}: keeps its status",
            held.email
        );
    }
    let alice = repos.users.get_global(alice.id).await.unwrap().unwrap();
    assert_eq!(
        (alice.role, alice.user_status),
        (UserRole::Admin, UserStatus::Suspended),
        "the role and status a run with the password would have reset stay"
    );
}
