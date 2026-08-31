// ABOUTME: PostgreSQL-lane test for the users create/update split — the engine where `--force` was broken
// ABOUTME: PG's `create` has always been a bare INSERT, so before `update` existed there was no way to write a row

//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `PostgreSQL` user write-split tests.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(feature = "postgresql")]

use pierre_core::models::User;
use pierre_database::database::test_utils::create_test_db;
use uuid::Uuid;

fn user(email: &str) -> User {
    User::new(
        email.to_owned(),
        "hash_not_verified".to_owned(),
        Some("PG Split Test".to_owned()),
    )
}

/// carnet#124: `users.create` was an upsert on `SQLite` and a bare INSERT on
/// `PostgreSQL` against `email TEXT UNIQUE`, so every caller that meant "write this
/// user back" — Firebase account linking, `pierre-cli user create --force` — worked
/// on a developer's machine and raised a duplicate-key error in production. `update`
/// is the missing half, and it has to exist on the engine production runs.
#[tokio::test]
async fn test_pg_create_refuses_a_duplicate_and_update_writes_the_row() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    let created = user("pg-split@dravr.ai");
    repos.users.create(&created).await.unwrap();

    // The duplicate is refused as structured input, not as a raw driver error.
    let err = repos
        .users
        .create(&user("pg-split@dravr.ai"))
        .await
        .expect_err("the unique email index refuses a second insert");
    assert!(
        err.to_string().contains("Email already in use"),
        "PG must map its unique violation to the same error SQLite gives: {err}"
    );

    // The write that used to be impossible on this engine.
    let mut loaded = repos
        .users
        .get_by_email("pg-split@dravr.ai")
        .await
        .unwrap()
        .expect("user created");
    loaded.firebase_uid = Some("pg-firebase-uid".to_owned());
    loaded.auth_provider = "google".to_owned();
    loaded.locale = "en".to_owned();
    loaded.is_admin = true;
    repos.users.update(&loaded).await.unwrap();

    let linked = repos
        .users
        .get_by_firebase_uid("pg-firebase-uid")
        .await
        .unwrap()
        .expect("the Firebase link resolves on PG");
    assert_eq!(linked.id, created.id);
    assert_eq!(linked.locale, "en");
    assert!(linked.is_admin);
    assert_eq!(linked.auth_provider, "google");

    // A row that does not exist is reported, not silently skipped.
    let mut ghost = user("pg-ghost@dravr.ai");
    ghost.id = Uuid::new_v4();
    repos
        .users
        .update(&ghost)
        .await
        .expect_err("no row carries that id");

    // `create` and `update` agree on the mutable column set.
    let mut fresh = user("pg-preferences@dravr.ai");
    fresh.timezone = Some("America/Montreal".to_owned());
    fresh.theme = Some("dark".to_owned());
    repos.users.create(&fresh).await.unwrap();
    let stored = repos
        .users
        .get_by_email("pg-preferences@dravr.ai")
        .await
        .unwrap()
        .expect("user created");
    assert_eq!(stored.timezone, Some("America/Montreal".to_owned()));
    assert_eq!(stored.theme, Some("dark".to_owned()));

    // `users` has a unique index on firebase_uid too, and Postgres names the
    // constraint — so the error says which one collided rather than blaming email.
    let mut one = user("pg-device-one@dravr.ai");
    one.firebase_uid = Some("pg-google-uid-42".to_owned());
    repos.users.create(&one).await.unwrap();
    let mut two = user("pg-device-two@dravr.ai");
    two.firebase_uid = Some("pg-google-uid-42".to_owned());
    let err = repos
        .users
        .create(&two)
        .await
        .expect_err("the firebase_uid index refuses the second insert")
        .to_string();
    assert!(
        err.contains("Firebase account already linked"),
        "PG must name the constraint that collided, got: {err}"
    );
}
