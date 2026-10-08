// ABOUTME: Covers the per-user Home preferences against whichever backend DATABASE_URL names
// ABOUTME: No row reads as the defaults, a write round-trips, a second write replaces, and one user never reads another's
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `user_home_preferences` holds what an athlete chose about Home's layout,
//! so the web and the phone agree (carnet#820). The statements are written
//! once and emitted for both backends, differing only in how the uuid reaches
//! `user_id`; `create_test_db` opens whichever `DATABASE_URL` names, so the
//! same assertions cover `SQLite` and `PostgreSQL`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_core::models::User;
use pierre_database::repositories::HomePreferences;
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
}

#[tokio::test]
async fn a_choice_round_trips_and_a_second_one_replaces_it() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(repos).await;
    let other = fresh_user(repos).await;

    repos
        .home_preferences
        .set_home_preferences(
            user_id,
            HomePreferences {
                plan_suggestion_hidden: true,
            },
        )
        .await
        .unwrap();
    assert!(
        repos
            .home_preferences
            .get_home_preferences(user_id)
            .await
            .unwrap()
            .plan_suggestion_hidden
    );
    // Scoped by user: the other athlete still has every default.
    assert_eq!(
        repos
            .home_preferences
            .get_home_preferences(other)
            .await
            .unwrap(),
        HomePreferences::default()
    );

    repos
        .home_preferences
        .set_home_preferences(
            user_id,
            HomePreferences {
                plan_suggestion_hidden: false,
            },
        )
        .await
        .unwrap();
    assert!(
        !repos
            .home_preferences
            .get_home_preferences(user_id)
            .await
            .unwrap()
            .plan_suggestion_hidden
    );
}
