// ABOUTME: Bootstrap seeder role test — the operator lands as super_admin, demo users as plain user
// ABOUTME: Guards the dev device-login approval path, which gates on the operator's super_admin role
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use pierre_core::permissions::UserRole;
use pierre_seeders::bootstrap::{self, SeedArgs};

/// The bootstrap seeder must create the operator (ADMIN_EMAIL) as `super_admin`, not a
/// plain `admin`. The browser device-login approval page gates on `is_super_admin`, and
/// cookie_admin_middleware derives console permissions from the role column — a plain
/// `admin` operator cannot approve a CLI sign-in or reach config/user/impersonation.
/// Demo users stay non-privileged.
#[tokio::test]
async fn bootstrap_operator_is_super_admin_demo_users_are_plain() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();

    bootstrap::run(
        SeedArgs {
            admin_email: "operator@dravr.ai".to_owned(),
            admin_password: "OperatorPass123!".to_owned(),
        },
        &repos,
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
        UserRole::SuperAdmin,
        "seeded operator must be super_admin so it can approve device logins"
    );
    assert!(operator.is_admin, "operator keeps the is_admin flag too");

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
    assert!(!alice.is_admin, "demo users are not admins");
}
