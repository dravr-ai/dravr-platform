// ABOUTME: Covers the single-column preference writes on users against whichever backend DATABASE_URL names
// ABOUTME: Round-trips every preference, pins the not-found contract, and holds set_theme to its clear semantics
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The six preference writes each set one column on `users` and report a
//! missing row as `NotFound` rather than a silent no-op. They share one body
//! across both backends, differing only in how a user id reaches the `id`
//! column — a `uuid` on `PostgreSQL`, `TEXT` on `SQLite`.
//!
//! `set_theme` had no test anywhere, which mattered because it is the only one
//! of the six that can legitimately write NULL: clearing the pin is a value,
//! not an absence of one, and a write that quietly kept the old scheme would
//! have read as success.
//!
//! These run on `SQLite` and on `PostgreSQL`: `create_test_db` opens whichever
//! `DATABASE_URL` names, so the same assertions cover both backends.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_core::models::{CoachingPersona, User};
use pierre_database::database::test_utils::create_test_db;
use uuid::Uuid;

/// A distinct user per test, so one test's writes cannot satisfy another's
/// assertions.
fn fresh_user() -> User {
    User::new(
        format!("prefs-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Preference Tester".to_owned()),
    )
}

#[tokio::test]
async fn each_preference_write_round_trips_its_own_column() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user = fresh_user();
    let user_id = repos.users.create(&user).await.unwrap();

    repos.users.update_locale(user_id, "fr").await.unwrap();
    repos
        .users
        .set_timezone(user_id, "America/Toronto")
        .await
        .unwrap();
    repos
        .users
        .set_coaching_persona(user_id, CoachingPersona::PowerAthlete)
        .await
        .unwrap();
    repos.users.set_manages_roster(user_id, true).await.unwrap();
    repos
        .users
        .update_analytics_consent(user_id, true)
        .await
        .unwrap();
    repos.users.set_theme(user_id, Some("dark")).await.unwrap();

    let read = repos
        .users
        .get_global(user_id)
        .await
        .unwrap()
        .expect("the user just created must read back");

    assert_eq!(read.locale, "fr");
    assert_eq!(read.timezone.as_deref(), Some("America/Toronto"));
    assert_eq!(read.coaching_persona, CoachingPersona::PowerAthlete);
    assert!(read.manages_roster, "the roster flag must persist as true");
    assert!(
        read.analytics_consent,
        "consent must persist as the caller set it"
    );
    assert_eq!(read.theme.as_deref(), Some("dark"));
}

#[tokio::test]
async fn set_theme_none_clears_a_pinned_scheme() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = repos.users.create(&fresh_user()).await.unwrap();

    repos.users.set_theme(user_id, Some("light")).await.unwrap();
    assert_eq!(
        repos
            .users
            .get_global(user_id)
            .await
            .unwrap()
            .expect("user must exist")
            .theme
            .as_deref(),
        Some("light")
    );

    repos.users.set_theme(user_id, None).await.unwrap();
    assert_eq!(
        repos
            .users
            .get_global(user_id)
            .await
            .unwrap()
            .expect("user must exist")
            .theme,
        None,
        "clearing the pin must write NULL, not keep the previous scheme"
    );
}

#[tokio::test]
async fn a_later_write_overwrites_an_earlier_one() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = repos.users.create(&fresh_user()).await.unwrap();

    repos.users.update_locale(user_id, "de").await.unwrap();
    repos.users.update_locale(user_id, "pt").await.unwrap();
    repos.users.set_manages_roster(user_id, true).await.unwrap();
    repos
        .users
        .set_manages_roster(user_id, false)
        .await
        .unwrap();

    let read = repos
        .users
        .get_global(user_id)
        .await
        .unwrap()
        .expect("user must exist");
    assert_eq!(read.locale, "pt");
    assert!(
        !read.manages_roster,
        "the second write must win, including when it writes false"
    );
}

#[tokio::test]
async fn a_preference_write_for_an_unknown_user_is_not_found() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let absent = Uuid::new_v4();

    let err = repos
        .users
        .set_theme(absent, Some("dark"))
        .await
        .expect_err("writing a preference for a user that does not exist must fail");
    assert!(
        err.to_string().contains(&absent.to_string()),
        "the error must name the user it could not find, got: {err}"
    );

    repos
        .users
        .update_locale(absent, "en")
        .await
        .expect_err("every preference write shares the not-found contract");
}
