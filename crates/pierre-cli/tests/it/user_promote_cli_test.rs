// ABOUTME: `user promote` on the host takes the approve-first rule the admin API's promotion takes (carnet#1589)
// ABOUTME: Runs the real binary on a scratch SQLite database: a pending or suspended account is refused, an approved one promoted

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::env;
use std::fs;
use std::process::{Command, Output};

use pierre_core::models::{User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::Database;
use uuid::Uuid;

use crate::scratch_db::open;

/// Base64 of 32 fixed bytes — the local KEK the binary bootstraps with.
const MASTER_KEY: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";

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

/// A plain account stored in `status`, as a sign-up leaves one.
async fn seed(database: &Database, email: &str, status: UserStatus) {
    let mut user = User::new(email.to_owned(), "not-a-login-path".to_owned(), None);
    user.user_status = status;
    database.repositories().users.create(&user).await.unwrap();
}

async fn role_of(database: &Database, email: &str) -> UserRole {
    let users = &database.repositories().users;
    users.get_by_email(email).await.unwrap().unwrap().role
}

/// `user promote` gives the role to an active account only. A pending
/// sign-up and a suspended account are refused, told to be approved first,
/// and keep the role they had; once approved, the same command promotes.
/// The command wrote the role itself before, whatever the status, so a
/// pending account left the host holding every power.
#[tokio::test]
async fn promote_refuses_an_account_that_is_not_active_and_promotes_it_once_approved() {
    let path = env::temp_dir().join(format!("pierre-cli-promote-{}.db", Uuid::new_v4()));
    let url = format!("sqlite:{}?mode=rwc", path.display());
    let run = |args: &[&str]| {
        Command::new(cli_binary())
            .env("DATABASE_URL", &url)
            .env("PIERRE_MASTER_ENCRYPTION_KEY", MASTER_KEY)
            .args(args)
            .output()
            .unwrap()
    };
    // Creates and migrates the database, and leaves one active admin.
    let created = run(&[
        "user",
        "create",
        "--email",
        "admin@example.com",
        "--password",
        "correct-horse-battery-staple",
    ]);
    assert!(created.status.success(), "{}", printed(&created));
    let database = open(&url).await;
    let (pending, suspended) = ("pending@example.com", "suspended@example.com");
    seed(&database, pending, UserStatus::Pending).await;
    seed(&database, suspended, UserStatus::Suspended).await;

    let mut refused = Vec::new();
    for (email, status) in [(pending, "pending"), (suspended, "suspended")] {
        let promoted = run(&["user", "promote", "--email", email]);
        refused.push((promoted, status, role_of(&database, email).await));
    }
    let approved = run(&["user", "approve", "--email", pending]);
    let promoted = run(&["user", "promote", "--email", pending]);
    let role_once_approved = role_of(&database, pending).await;
    drop(database);
    // Best-effort cleanup of the temp file.
    drop(fs::remove_file(&path));

    for (output, status, role) in &refused {
        let said = printed(output);
        assert!(
            !output.status.success(),
            "a {status} account is not promoted: {said}"
        );
        assert!(
            said.contains(&format!("this one is {status}")),
            "the refusal names the status: {said}"
        );
        assert!(
            said.contains("Approve the account first, then promote it."),
            "{said}"
        );
        assert_eq!(
            *role,
            UserRole::User,
            "a refused {status} account keeps its role"
        );
    }
    assert!(approved.status.success(), "{}", printed(&approved));
    assert!(promoted.status.success(), "{}", printed(&promoted));
    assert_eq!(role_once_approved, UserRole::Admin);
}
