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
use pierre_core::models::UserStatus;
use pierre_core::permissions::UserRole;
use pierre_database::RepositoryRegistry;
use pierre_seeders::bootstrap::{self, SeedArgs};
use serde_json::Value;

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

    bootstrap::run(
        SeedArgs {
            admin_email: "operator@dravr.ai".to_owned(),
            admin_password: "OperatorPass123!".to_owned(),
        },
        repos,
    )
    .await
    .unwrap();

    let operator = repos
        .users
        .get_by_email("operator@dravr.ai")
        .await
        .unwrap()
        .expect("operator seeded");
    assert_eq!(
        operator.role,
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

/// A seeding run in a fresh database, as `pierre-cli seed bootstrap` does it.
async fn seed(repos: &RepositoryRegistry) -> AppResult<()> {
    bootstrap::run(
        SeedArgs {
            admin_email: "operator@dravr.ai".to_owned(),
            admin_password: "OperatorPass123!".to_owned(),
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
    let operator = by_email("operator@dravr.ai").await;
    repos.users.set_admin_status(alice.id, true).await.unwrap();
    repos
        .users
        .set_admin_status(operator.id, false)
        .await
        .unwrap();
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

    assert_eq!(by_email("operator@dravr.ai").await.role, UserRole::Admin);
    assert_eq!(by_email("alice@demo.pierre.dev").await.role, UserRole::User);
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
    let operator = repos
        .users
        .get_by_email("operator@dravr.ai")
        .await
        .unwrap()
        .expect("seeded");
    repos.users.set_admin_status(alice.id, true).await.unwrap();
    repos
        .users
        .update_status(operator.id, UserStatus::Suspended, None)
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
