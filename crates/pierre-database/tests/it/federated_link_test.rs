// ABOUTME: carnet#1589 — linking a federated identity writes the columns the link owns and no other
// ABOUTME: Runs on whichever backend DATABASE_URL names: the account's role, status and activity are left as an admin set them

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::models::{User, UserStatus, FEDERATED_ONLY_PASSWORD_HASH};
use pierre_core::permissions::UserRole;
use pierre_database::repositories::FederatedLink;
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use pierre_test_support::server::{create_test_admin_user, create_test_user};
use uuid::Uuid;

const PASSWORD_HASH: &str = "the-hash-a-registration-stored";

/// An active account registered with a password, linked to `firebase_uid`.
async fn account(repos: &RepositoryRegistry, email: &str, firebase_uid: Option<&str>) -> User {
    let mut user = create_test_user(email, None);
    user.user_status = UserStatus::Active;
    PASSWORD_HASH.clone_into(&mut user.password_hash);
    user.firebase_uid = firebase_uid.map(str::to_owned);
    "email".clone_into(&mut user.auth_provider);
    repos.users.create(&user).await.unwrap();
    user
}

/// Link `user_id` to a `google.com` sign-in carrying `firebase_uid`; the
/// row as the link left it.
async fn link(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    firebase_uid: Option<&str>,
    retire: bool,
) -> User {
    repos
        .users
        .link_federated_identity(
            user_id,
            &FederatedLink {
                firebase_uid,
                auth_provider: "google.com",
                retired_password_hash: retire.then_some(FEDERATED_ONLY_PASSWORD_HASH),
            },
        )
        .await
        .unwrap()
        .expect("the account exists")
}

/// The link records the provider and the sign-in's Firebase UID. With the
/// account's credentials kept, the password stays and a sign-in that
/// carries no UID leaves an earlier one; with them retired, the password is
/// the federated-only marker and the UID is the sign-in's alone, an earlier
/// link dropped. The row returned is the stored one, and an id that is no
/// account links nothing.
#[tokio::test]
async fn a_link_writes_the_identity_and_keeps_or_retires_the_credentials() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    let kept = account(repos, "kept@example.com", Some("fb-earlier")).await;
    let linked = link(repos, kept.id, None, false).await;
    assert_eq!(linked.auth_provider, "google.com");
    assert_eq!(linked.firebase_uid.as_deref(), Some("fb-earlier"));
    assert_eq!(linked.password_hash, PASSWORD_HASH);
    let linked = link(repos, kept.id, Some("fb-new"), false).await;
    assert_eq!(linked.firebase_uid.as_deref(), Some("fb-new"));
    assert_eq!(linked.password_hash, PASSWORD_HASH);

    let dropped = account(repos, "dropped@example.com", Some("fb-squatter")).await;
    let linked = link(repos, dropped.id, None, true).await;
    assert_eq!(linked.firebase_uid, None, "an earlier link is dropped");
    assert_eq!(linked.password_hash, FEDERATED_ONLY_PASSWORD_HASH);
    assert_eq!(linked.auth_provider, "google.com");

    let replaced = account(repos, "replaced@example.com", Some("fb-squatter-2")).await;
    let linked = link(repos, replaced.id, Some("fb-owner"), true).await;
    assert_eq!(linked.firebase_uid.as_deref(), Some("fb-owner"));
    assert_eq!(linked.password_hash, FEDERATED_ONLY_PASSWORD_HASH);
    let stored = repos.users.get_global(replaced.id).await.unwrap().unwrap();
    assert_eq!(
        (
            stored.firebase_uid,
            stored.password_hash,
            stored.auth_provider
        ),
        (
            linked.firebase_uid,
            linked.password_hash,
            linked.auth_provider
        ),
        "the row returned is the row stored"
    );

    let no_one = repos
        .users
        .link_federated_identity(
            Uuid::new_v4(),
            &FederatedLink {
                firebase_uid: Some("fb-no-one"),
                auth_provider: "google.com",
                retired_password_hash: None,
            },
        )
        .await
        .unwrap();
    assert!(no_one.is_none(), "an id that is no account links nothing");
}

/// The link writes nothing of the account's standing. An account suspended,
/// demoted or promoted after a sign-in read it keeps what the admin wrote,
/// with its credentials kept or retired, and the row the link returns says
/// so: the sign-in goes on with the standing as it is now. The whole row
/// written back from the earlier read put the old standing back.
#[tokio::test]
async fn a_link_leaves_the_standing_an_admin_wrote_since_the_account_was_read() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    // A second admin in good standing, so a demotion is never the last one's.
    let standing = create_test_admin_user("standing@example.com", None);
    repos.users.create(&standing).await.unwrap();

    for retire in [false, true] {
        let suspended = account(repos, &format!("suspended-{retire}@example.com"), None).await;
        repos
            .users
            .update_status(suspended.id, UserStatus::Suspended, None)
            .await
            .unwrap();
        let linked = link(repos, suspended.id, Some(&format!("fb-s-{retire}")), retire).await;
        assert_eq!(linked.user_status, UserStatus::Suspended, "retire {retire}");
        assert_eq!(linked.role, UserRole::User);

        let promoted = account(repos, &format!("promoted-{retire}@example.com"), None).await;
        repos
            .users
            .set_admin_status(promoted.id, true)
            .await
            .unwrap();
        let linked = link(repos, promoted.id, Some(&format!("fb-p-{retire}")), retire).await;
        assert_eq!(linked.role, UserRole::Admin, "retire {retire}");
        assert_eq!(linked.user_status, UserStatus::Active);

        repos
            .users
            .set_admin_status(promoted.id, false)
            .await
            .unwrap();
        let linked = link(repos, promoted.id, None, retire).await;
        assert_eq!(linked.role, UserRole::User, "retire {retire}");
        let stored = repos.users.get_global(promoted.id).await.unwrap().unwrap();
        assert_eq!(
            (stored.role, stored.user_status),
            (linked.role, linked.user_status)
        );
    }
}
