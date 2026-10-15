// ABOUTME: Covers the per-user unit choice against whichever backend DATABASE_URL names
// ABOUTME: No row reads metric, a stored choice replaces the last one, and one user never reads another's
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `user_unit_preferences` holds the unit system an athlete chose in Settings
//! (carnet#835); with no choice they read metric (carnet#1044). The statements
//! are written once and emitted for both backends; `create_test_db` opens
//! whichever `DATABASE_URL` names, so the same assertions cover `SQLite` and
//! `PostgreSQL`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_core::models::{UnitSystem, User};
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

/// A distinct user per call; the table's foreign key needs the row to exist.
async fn fresh_user(repos: &RepositoryRegistry) -> Uuid {
    let user = User::new(
        format!("unit-prefs-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Unit Preferences Tester".to_owned()),
    );
    repos.users.create(&user).await.unwrap()
}

#[tokio::test]
async fn a_user_with_nothing_stored_reads_metric() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(repos).await;

    let units = repos.unit_preferences.get_units(user_id).await.unwrap();
    assert_eq!(units, UnitSystem::Metric);
}

#[tokio::test]
async fn a_stored_choice_replaces_the_last_and_stays_with_its_user() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(repos).await;
    let other = fresh_user(repos).await;
    let prefs = repos.unit_preferences.as_ref();

    prefs
        .set_units(user_id, UnitSystem::Imperial)
        .await
        .unwrap();
    assert_eq!(
        prefs.get_units(user_id).await.unwrap(),
        UnitSystem::Imperial
    );

    prefs.set_units(user_id, UnitSystem::Metric).await.unwrap();
    assert_eq!(prefs.get_units(user_id).await.unwrap(), UnitSystem::Metric);

    prefs
        .set_units(user_id, UnitSystem::Imperial)
        .await
        .unwrap();
    assert_eq!(prefs.get_units(other).await.unwrap(), UnitSystem::Metric);
}
