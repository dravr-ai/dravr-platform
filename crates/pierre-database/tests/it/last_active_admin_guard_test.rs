// ABOUTME: carnet#1574 — the last admin in good standing cannot be removed, decided inside the write's own transaction
// ABOUTME: The account deletion refuses it past the service's early check, and two removals racing each other leave one admin

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::errors::ErrorCode;
use pierre_core::models::UserStatus;
use pierre_core::permissions::UserRole;
use pierre_test_support::account::delete_account;
use pierre_test_support::db::{create_concurrent_test_db, create_test_db};
use pierre_test_support::server::create_test_admin_user;

/// The account deletion decides inside its own transaction: called straight
/// on the repository, past the service's early check, it still refuses the
/// last active admin, with the text and class the service refuses with, and
/// deletes nothing.
#[tokio::test]
async fn the_account_deletion_itself_refuses_the_last_active_admin() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let last = create_test_admin_user("only_admin@example.com", Some("Only".to_owned()));
    repos.users.create(&last).await.unwrap();
    let mut held = create_test_admin_user("held_admin@example.com", Some("Held".to_owned()));
    held.user_status = UserStatus::Suspended;
    repos.users.create(&held).await.unwrap();

    let error = delete_account(repos, last.id, None)
        .await
        .expect_err("the last active admin must not be deleted");
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert_eq!(error.message, "Cannot delete the last active admin");
    assert!(
        repos.users.get_global(last.id).await.unwrap().is_some(),
        "the refused deletion removed nothing"
    );

    // An admin who is not in good standing is deleted: it was never the one
    // the platform relies on. So is the first, once a second stands.
    delete_account(repos, held.id, None).await.unwrap();
    let second = create_test_admin_user("relief_admin@example.com", Some("Relief".to_owned()));
    repos.users.create(&second).await.unwrap();
    delete_account(repos, last.id, None).await.unwrap();
    assert!(repos.users.get_global(last.id).await.unwrap().is_none());
    // And now `second` is the last one.
    let error = delete_account(repos, second.id, None).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
}

/// How many times each race below is run: the window between one writer's
/// check and its write is narrow, so one round proves little.
const ADMIN_RACE_ROUNDS: usize = 12;

/// Two admins in good standing and two writes at once, each taking a
/// different one out of that standing. Checked and then written, both writes
/// found the other admin still standing and both landed, leaving nobody to
/// manage the platform. One transaction each, under the lock, exactly one
/// lands and the other is refused as the last active admin.
#[tokio::test]
async fn two_admin_removals_racing_each_other_leave_one_admin_standing() {
    for round in 0..ADMIN_RACE_ROUNDS {
        // Connections that overlap in the engine, as a deployed pool's do:
        // the default one-connection SQLite pool queues the two writers and
        // never reaches the window. A database per round, so each one starts
        // from exactly two admins.
        let db = create_concurrent_test_db().await.unwrap();
        let repos = db.repositories();
        let users = &repos.users;
        let first = create_test_admin_user("first_admin@example.com", None);
        let second = create_test_admin_user("second_admin@example.com", None);
        users.create(&first).await.unwrap();
        users.create(&second).await.unwrap();
        let mut rewritten = first.clone();
        rewritten.role = UserRole::User;
        // Two connections open before the race: a pool opens its second one
        // on demand, and a racer still connecting lets the other finish first.
        let (warm, also_warm) = tokio::join!(users.list_admins(), users.list_admins());
        assert_eq!(warm.unwrap().len() + also_warm.unwrap().len(), 4);

        // Each round races a different pair of the four writes that can
        // take an admin out of good standing.
        let (one, other) = match round % 4 {
            0 => tokio::join!(
                async { users.set_admin_status(first.id, false).await.map(drop) },
                async { users.set_admin_status(second.id, false).await.map(drop) },
            ),
            1 => tokio::join!(async { users.update(&rewritten).await.map(drop) }, async {
                users
                    .update_status(second.id, UserStatus::Suspended, None)
                    .await
                    .map(drop)
            },),
            2 => tokio::join!(
                async { delete_account(repos, first.id, None).await.map(drop) },
                async { users.set_admin_status(second.id, false).await.map(drop) },
            ),
            _ => tokio::join!(
                async { delete_account(repos, first.id, None).await.map(drop) },
                async { delete_account(repos, second.id, None).await.map(drop) },
            ),
        };

        let refusals: Vec<_> = [&one, &other]
            .into_iter()
            .filter_map(|outcome| outcome.as_ref().err())
            .collect();
        assert_eq!(
            refusals.len(),
            1,
            "round {round}: exactly one of the two removals lands: {one:?} / {other:?}"
        );
        assert_eq!(refusals[0].code, ErrorCode::InvalidInput, "round {round}");
        assert!(
            refusals[0].message.contains("the last active admin"),
            "round {round}: {}",
            refusals[0].message
        );
        let standing = users
            .list_admins()
            .await
            .unwrap()
            .iter()
            .filter(|admin| admin.is_active_admin())
            .count();
        assert_eq!(
            standing, 1,
            "round {round}: one admin is left in good standing"
        );
    }
}
