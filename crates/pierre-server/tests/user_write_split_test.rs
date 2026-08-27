// ABOUTME: `users.create` inserts and `users.update` writes — one method no longer means both
// ABOUTME: Guards Firebase account linking, which needs firebase_uid written onto an existing row

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use pierre_core::models::User;
use uuid::Uuid;

fn user(email: &str) -> User {
    User::new(
        email.to_owned(),
        "hash_not_verified".to_owned(),
        Some("Split Test".to_owned()),
    )
}

/// `create` is insert-only. It used to look the email up and fall through to an
/// UPDATE, so a second call silently overwrote the account — including its password
/// hash and admin flag. A duplicate email is now refused, the same way `PostgreSQL`'s
/// unique index has always refused it.
#[tokio::test]
async fn create_refuses_an_email_that_is_already_taken() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();

    let first = user("taken@dravr.ai");
    repos.users.create(&first).await.unwrap();

    let mut second = user("taken@dravr.ai");
    second.password_hash = "a_different_hash".to_owned();
    second.is_admin = true;
    let err = repos
        .users
        .create(&second)
        .await
        .expect_err("a duplicate email must be refused, not silently applied");
    assert!(
        err.to_string().contains("Email already in use"),
        "expected the structured duplicate-email error, got: {err}"
    );

    let stored = repos
        .users
        .get_by_email("taken@dravr.ai")
        .await
        .unwrap()
        .expect("the first user survives");
    assert_eq!(stored.id, first.id, "the original row is untouched");
    assert_eq!(stored.password_hash, "hash_not_verified");
    assert!(!stored.is_admin, "a refused create grants no privileges");
}

/// The linking path in `AuthService` loads a user by email, stamps the Firebase UID
/// on it and writes it back. The old upsert's UPDATE branch listed neither
/// `firebase_uid` nor `auth_provider`, so the link never took and every subsequent
/// sign-in re-ran the same branch.
#[tokio::test]
async fn update_writes_the_firebase_link_onto_an_existing_row() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();

    let created = user("linkme@dravr.ai");
    repos.users.create(&created).await.unwrap();

    let mut loaded = repos
        .users
        .get_by_email("linkme@dravr.ai")
        .await
        .unwrap()
        .expect("user created");
    assert_eq!(loaded.firebase_uid, None, "nothing linked yet");

    loaded.firebase_uid = Some("firebase-uid-123".to_owned());
    loaded.auth_provider = "google".to_owned();
    repos.users.update(&loaded).await.unwrap();

    let linked = repos
        .users
        .get_by_firebase_uid("firebase-uid-123")
        .await
        .unwrap()
        .expect("the UID now resolves — this is what the old upsert never wrote");
    assert_eq!(linked.id, created.id);
    assert_eq!(linked.auth_provider, "google");
    assert_eq!(
        linked.email, "linkme@dravr.ai",
        "linking keeps the account, it does not mint a second one"
    );
}

/// `update` carries every mutable column, because its callers change several at
/// once — `pierre-cli user create --force` re-asserts password, role and locale
/// together.
#[tokio::test]
async fn update_writes_the_whole_mutable_row() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();

    let created = user("rewrite@dravr.ai");
    repos.users.create(&created).await.unwrap();

    let mut loaded = repos
        .users
        .get_by_email("rewrite@dravr.ai")
        .await
        .unwrap()
        .expect("user created");
    loaded.display_name = Some("Renamed".to_owned());
    loaded.password_hash = "rehashed".to_owned();
    loaded.locale = "en".to_owned();
    loaded.is_admin = true;
    loaded.manages_roster = true;
    loaded.timezone = Some("America/Montreal".to_owned());
    repos.users.update(&loaded).await.unwrap();

    let after = repos
        .users
        .get_by_email("rewrite@dravr.ai")
        .await
        .unwrap()
        .expect("user still there");
    assert_eq!(after.display_name, Some("Renamed".to_owned()));
    assert_eq!(after.password_hash, "rehashed");
    assert_eq!(after.locale, "en");
    assert!(after.is_admin);
    assert!(after.manages_roster);
    assert_eq!(after.timezone, Some("America/Montreal".to_owned()));
    assert_eq!(
        after.created_at.timestamp(),
        created.created_at.timestamp(),
        "created_at is not a mutable fact"
    );
}

/// `update` matches on id, so a caller holding a `User` for a deleted or never-created
/// account is told so rather than writing nothing and reporting success.
#[tokio::test]
async fn update_reports_a_missing_row_instead_of_silently_doing_nothing() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();

    let mut ghost = user("ghost@dravr.ai");
    ghost.id = Uuid::new_v4();

    let err = repos
        .users
        .update(&ghost)
        .await
        .expect_err("no row carries that id");
    assert!(
        err.to_string().contains("not found") || err.to_string().contains("User "),
        "expected a not-found error, got: {err}"
    );
}

/// `create` and `update` have to agree on which columns are mutable, or a caller that
/// sets one on a brand-new user silently loses it — `create` writes NULL and there is
/// no upsert fallback to catch it on the next call.
#[tokio::test]
async fn create_carries_every_column_update_writes() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();

    let mut fresh = user("preferences@dravr.ai");
    fresh.timezone = Some("America/Montreal".to_owned());
    fresh.theme = Some("dark".to_owned());
    fresh.locale = "en".to_owned();
    repos.users.create(&fresh).await.unwrap();

    let stored = repos
        .users
        .get_by_email("preferences@dravr.ai")
        .await
        .unwrap()
        .expect("user created");
    assert_eq!(
        stored.timezone,
        Some("America/Montreal".to_owned()),
        "create dropped timezone, which update writes"
    );
    assert_eq!(stored.theme, Some("dark".to_owned()));
    assert_eq!(stored.locale, "en");
}

/// `users` carries a unique index on `firebase_uid` as well as on `email`. Reporting
/// both as "Email already in use" sends whoever reads the log hunting a column that
/// did not collide.
#[tokio::test]
async fn a_duplicate_firebase_uid_is_not_reported_as_an_email_collision() {
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();

    let mut first = user("device-one@dravr.ai");
    first.firebase_uid = Some("google-uid-42".to_owned());
    repos.users.create(&first).await.unwrap();

    // A different address, the same Firebase account — the race two concurrent
    // sign-ins for one UID lose.
    let mut second = user("device-two@dravr.ai");
    second.firebase_uid = Some("google-uid-42".to_owned());
    let err = repos
        .users
        .create(&second)
        .await
        .expect_err("the firebase_uid index refuses the second insert")
        .to_string();

    assert!(
        err.contains("Firebase account already linked"),
        "the error must name the constraint that collided, got: {err}"
    );
    assert!(
        !err.contains("Email already in use"),
        "the emails did not collide, so the message must not say they did: {err}"
    );
}
