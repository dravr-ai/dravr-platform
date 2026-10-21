// ABOUTME: Covers the per-user Home preferences against whichever backend DATABASE_URL names
// ABOUTME: No row reads as the defaults, a write round-trips, a partial write keeps the other choice, one user never reads another's
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `user_home_preferences` holds what an athlete chose about Home's layout,
//! so the web and the phone agree (carnet#820, carnet#1098). The statements are written
//! once and emitted for both backends, differing only in how the uuid reaches
//! `user_id`; `create_test_db` opens whichever `DATABASE_URL` names, so the
//! same assertions cover `SQLite` and `PostgreSQL`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_core::models::User;
use pierre_database::repositories::{HomePreferences, HomePreferencesUpdate};
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

/// A distinct user per call; the table's foreign key needs the row to exist.
async fn fresh_user(repos: &RepositoryRegistry) -> Uuid {
    let user = User::new(
        format!("home-prefs-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Home Preferences Tester".to_owned()),
    );
    repos.users.create(&user).await.unwrap()
}

#[tokio::test]
async fn a_user_with_no_choice_reads_the_defaults() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(repos).await;

    let prefs = repos
        .home_preferences
        .get_home_preferences(user_id)
        .await
        .unwrap();
    assert_eq!(prefs, HomePreferences::default());
    assert!(!prefs.plan_suggestion_hidden);
    assert!(!prefs.getting_started_hidden);
}

async fn read(repos: &RepositoryRegistry, user_id: Uuid) -> HomePreferences {
    repos
        .home_preferences
        .get_home_preferences(user_id)
        .await
        .unwrap()
}

async fn write(repos: &RepositoryRegistry, user_id: Uuid, update: HomePreferencesUpdate) {
    repos
        .home_preferences
        .set_home_preferences(user_id, update)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_choice_round_trips_and_a_second_one_replaces_it() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(repos).await;
    let other = fresh_user(repos).await;

    write(
        repos,
        user_id,
        HomePreferencesUpdate {
            plan_suggestion_hidden: Some(true),
            getting_started_hidden: Some(true),
        },
    )
    .await;
    assert_eq!(
        read(repos, user_id).await,
        HomePreferences {
            plan_suggestion_hidden: true,
            getting_started_hidden: true,
        }
    );
    // Scoped by user: the other athlete still has every default.
    assert_eq!(read(repos, other).await, HomePreferences::default());

    write(
        repos,
        user_id,
        HomePreferencesUpdate {
            plan_suggestion_hidden: Some(false),
            getting_started_hidden: Some(false),
        },
    )
    .await;
    assert_eq!(read(repos, user_id).await, HomePreferences::default());
}

#[tokio::test]
async fn a_write_without_a_field_keeps_what_is_stored() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(repos).await;

    // First write names only Getting started: the row is created with the
    // plan suggestion at its default.
    write(
        repos,
        user_id,
        HomePreferencesUpdate {
            plan_suggestion_hidden: None,
            getting_started_hidden: Some(true),
        },
    )
    .await;
    assert_eq!(
        read(repos, user_id).await,
        HomePreferences {
            plan_suggestion_hidden: false,
            getting_started_hidden: true,
        }
    );

    // A client that predates Getting started writes the plan choice alone:
    // the hidden Getting started survives it.
    write(
        repos,
        user_id,
        HomePreferencesUpdate {
            plan_suggestion_hidden: Some(true),
            getting_started_hidden: None,
        },
    )
    .await;
    assert_eq!(
        read(repos, user_id).await,
        HomePreferences {
            plan_suggestion_hidden: true,
            getting_started_hidden: true,
        }
    );

    // An empty write changes nothing.
    write(repos, user_id, HomePreferencesUpdate::default()).await;
    assert_eq!(
        read(repos, user_id).await,
        HomePreferences {
            plan_suggestion_hidden: true,
            getting_started_hidden: true,
        }
    );
}
