// ABOUTME: `user create --role user` makes a plain active account; without the flag the command makes an admin, as before
// ABOUTME: Runs the real binary on a scratch SQLite database and reads the stored role, status and tier back

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::env;
use std::fs;
use std::process::{Command, Output};

use pierre_core::constants::tiers;
use pierre_core::models::{User, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::Database;
use uuid::Uuid;

use crate::scratch_db::open;

/// Base64 of 32 fixed bytes — the local KEK the binary bootstraps with.
const MASTER_KEY: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";

/// The password every account of this file is created with.
const PASSWORD: &str = "correct-horse-battery-staple";

fn cli_binary() -> String {
    env::var("CARGO_BIN_EXE_pierre-cli")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_pierre-cli").to_owned())
}

/// Everything the binary printed, stdout then stderr.
fn printed(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// The stored account behind `email`.
async fn stored(database: &Database, email: &str) -> User {
    let users = &database.repositories().users;
    users.get_by_email(email).await.unwrap().unwrap()
}

/// `user create` made an admin whatever it was asked, so nothing on the
/// host could make the plain account a test signs in with to be refused an
/// admin route (carnet#1589). `--role user` makes one: active, a plain
/// user on the starter tier, with a tenant of its own. Without the flag,
/// and with `--role admin`, the command makes the admin it always made.
///
/// The flag creates and re-asserts plain accounts; it is not a second way
/// to demote. Asked of an existing admin with `--force`, it is refused,
/// names `user demote`, and writes nothing. An existing plain account is
/// re-asserted, keeping its role. A role that is neither is refused.
#[tokio::test]
async fn create_makes_a_plain_account_on_request_and_an_admin_otherwise() {
    let path = env::temp_dir().join(format!("pierre-cli-create-role-{}.db", Uuid::new_v4()));
    let url = format!("sqlite:{}?mode=rwc", path.display());
    let run = |args: &[&str]| {
        Command::new(cli_binary())
            .env("DATABASE_URL", &url)
            .env("PIERRE_MASTER_ENCRYPTION_KEY", MASTER_KEY)
            .args(args)
            .output()
            .unwrap()
    };
    let create = |email: &str, extra: &[&str]| {
        let mut args = vec!["user", "create", "--email", email, "--password", PASSWORD];
        args.extend_from_slice(extra);
        run(&args)
    };
    let (admin, named_admin, plain) = (
        "admin@example.com",
        "named-admin@example.com",
        "plain@example.com",
    );

    let default_role = create(admin, &[]);
    let asked_admin = create(named_admin, &["--role", "admin"]);
    let asked_user = create(plain, &["--role", "user"]);
    let unknown_role = create("owner@example.com", &["--role", "owner"]);
    let demotion = create(admin, &["--role", "user", "--force"]);
    let reasserted = create(
        plain,
        &["--role", "user", "--force", "--name", "Plain Again"],
    );

    assert!(default_role.status.success(), "{}", printed(&default_role));
    let database = open(&url).await;
    let admin_account = stored(&database, admin).await;
    let named_admin_account = database
        .repositories()
        .users
        .get_by_email(named_admin)
        .await
        .unwrap();
    let plain_account = database
        .repositories()
        .users
        .get_by_email(plain)
        .await
        .unwrap();
    let plain_tenants = match &plain_account {
        Some(account) => database
            .repositories()
            .tenants
            .list_for_user(account.id)
            .await
            .unwrap(),
        None => Vec::new(),
    };
    let owner_account = database
        .repositories()
        .users
        .get_by_email("owner@example.com")
        .await
        .unwrap();
    drop(database);
    // Best-effort cleanup of the temp file.
    drop(fs::remove_file(&path));

    // Without the flag: the admin the command always made.
    assert_eq!(admin_account.role, UserRole::Admin);
    assert_eq!(admin_account.user_status, UserStatus::Active);
    assert_eq!(admin_account.tier, UserTier::Enterprise);

    assert!(asked_admin.status.success(), "{}", printed(&asked_admin));
    let named_admin_account = named_admin_account.expect("--role admin creates the account");
    assert_eq!(named_admin_account.role, UserRole::Admin);

    assert!(asked_user.status.success(), "{}", printed(&asked_user));
    let plain_account = plain_account.expect("--role user creates the account");
    assert_eq!(plain_account.role, UserRole::User, "a plain account");
    assert_eq!(plain_account.user_status, UserStatus::Active);
    assert!(plain_account.is_active);
    assert_eq!(plain_account.tier, UserTier::Starter);
    assert_eq!(
        plain_account.display_name.as_deref(),
        Some("Plain Again"),
        "--force re-asserts a plain account"
    );
    assert!(reasserted.status.success(), "{}", printed(&reasserted));
    assert_eq!(plain_tenants.len(), 1, "with a tenant of its own");
    assert_eq!(plain_tenants[0].owner_user_id, plain_account.id);
    assert_eq!(
        plain_tenants[0].plan,
        tiers::STARTER,
        "on the plan a registration starts on"
    );

    let said = printed(&unknown_role);
    assert!(!unknown_role.status.success(), "{said}");
    assert!(said.contains("owner"), "the refusal names the role: {said}");
    assert!(owner_account.is_none(), "and creates no account");

    let said = printed(&demotion);
    assert!(
        !demotion.status.success(),
        "--role user does not demote an admin: {said}"
    );
    assert!(said.contains("user demote"), "it names the command: {said}");
    assert_eq!(
        admin_account.role,
        UserRole::Admin,
        "the admin keeps its role"
    );
}
