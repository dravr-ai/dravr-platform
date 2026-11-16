// ABOUTME: carnet#1589 — a status write that names the status it expects lands only on an account still in it
// ABOUTME: A decided account is left as it was, and two decisions racing on one pending sign-up land once

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::models::{User, UserStatus};
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::{create_concurrent_test_db, create_test_db};
use pierre_test_support::server::create_test_user;
use uuid::Uuid;

/// A plain account stored in `status`, approved by no one at no time.
async fn account_in(repos: &RepositoryRegistry, email: &str, status: UserStatus) -> User {
    let mut user = create_test_user(email, None);
    user.user_status = status;
    user.approved_at = None;
    repos.users.create(&user).await.unwrap();
    user
}

async fn stored(repos: &RepositoryRegistry, id: Uuid) -> User {
    repos.users.get_global(id).await.unwrap().unwrap()
}

/// A write that expects a pending account changes nothing on one that is
/// active or suspended, or on an id that is no account, and says so with
/// `None`. On a pending one it is the plain status write.
#[tokio::test]
async fn a_guarded_status_write_lands_only_on_an_account_still_in_that_status() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let users = &repos.users;
    let approved = account_in(repos, "approved-since@example.com", UserStatus::Active).await;
    let suspended = account_in(repos, "suspended-since@example.com", UserStatus::Suspended).await;
    let waiting = account_in(repos, "waiting@example.com", UserStatus::Pending).await;

    let rejected = users
        .update_status_from(
            approved.id,
            UserStatus::Pending,
            UserStatus::Suspended,
            None,
        )
        .await
        .unwrap();
    assert!(rejected.is_none(), "an active account is not a sign-up");
    assert_eq!(
        stored(repos, approved.id).await.user_status,
        UserStatus::Active
    );

    let reinstated = users
        .update_status_from(suspended.id, UserStatus::Pending, UserStatus::Active, None)
        .await
        .unwrap();
    assert!(reinstated.is_none(), "a suspended account is not a sign-up");
    let still_suspended = stored(repos, suspended.id).await;
    assert_eq!(still_suspended.user_status, UserStatus::Suspended);
    assert_eq!(still_suspended.approved_at, None, "no approval was stamped");

    let no_one = users
        .update_status_from(
            Uuid::new_v4(),
            UserStatus::Pending,
            UserStatus::Active,
            None,
        )
        .await
        .unwrap();
    assert!(no_one.is_none());

    let welcomed = users
        .update_status_from(waiting.id, UserStatus::Pending, UserStatus::Active, None)
        .await
        .unwrap()
        .expect("a pending account is decided");
    assert_eq!(welcomed.user.user_status, UserStatus::Active);
    assert!(welcomed.user.approved_at.is_some());
    assert_eq!(
        stored(repos, waiting.id).await.user_status,
        UserStatus::Active
    );
}

/// How many times the race below is run: one round proves little about a
/// window this narrow.
const DECISION_RACE_ROUNDS: usize = 12;

/// An approval and a rejection of one pending sign-up at once, as two
/// reviewers tapping the two buttons of its card. Checked first and written
/// after, both passed the check and the rejection suspended the account the
/// approval had just made active. With the status in the statement, exactly
/// one of them finds a pending row, and the account is what that one made it.
#[tokio::test]
async fn two_decisions_racing_on_one_pending_sign_up_land_once() {
    for round in 0..DECISION_RACE_ROUNDS {
        let db = create_concurrent_test_db().await.unwrap();
        let repos = db.repositories();
        let users = &repos.users;
        let waiting = account_in(repos, "contested@example.com", UserStatus::Pending).await;
        // Two connections open before the race: a pool opens its second one
        // on demand, and a racer still connecting lets the other finish first.
        let (warm, also_warm) = tokio::join!(users.count(), users.count());
        assert_eq!(warm.unwrap() + also_warm.unwrap(), 2);

        let (approval, rejection) = tokio::join!(
            users.update_status_from(waiting.id, UserStatus::Pending, UserStatus::Active, None),
            users.update_status_from(waiting.id, UserStatus::Pending, UserStatus::Suspended, None),
        );
        let (approval, rejection) = (approval.unwrap(), rejection.unwrap());

        let expected = match (&approval, &rejection) {
            (Some(_), None) => UserStatus::Active,
            (None, Some(_)) => UserStatus::Suspended,
            _ => panic!(
                "round {round}: exactly one decision lands, got approval={} rejection={}",
                approval.is_some(),
                rejection.is_some()
            ),
        };
        assert_eq!(
            stored(repos, waiting.id).await.user_status,
            expected,
            "round {round}: the account is what the one decision made it"
        );
    }
}
