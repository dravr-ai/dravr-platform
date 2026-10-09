// ABOUTME: Covers the per-user unit preference inputs against whichever backend DATABASE_URL names
// ABOUTME: No row reads as automatic, each input is written alone without clearing the others, and one user never reads another's
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `user_unit_preferences` holds the three inputs that decide whether an
//! athlete reads kilometres or miles (carnet#835): their Settings choice, the
//! provider's own setting and their device locale. The statements are written
//! once and emitted for both backends; `create_test_db` opens whichever
//! `DATABASE_URL` names, so the same assertions cover `SQLite` and `PostgreSQL`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_core::models::{UnitPreference, UnitSystem, User};
use pierre_database::repositories::StoredUnitPreferences;
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
async fn a_user_with_nothing_stored_reads_automatic() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(repos).await;

    let stored = repos
        .unit_preferences
        .get_unit_preferences(user_id)
        .await
        .unwrap();
    assert_eq!(stored, StoredUnitPreferences::default());
    assert_eq!(stored.preference, UnitPreference::Automatic);
}

#[tokio::test]
async fn each_input_is_written_alone_and_never_clears_the_others() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(repos).await;
    let other = fresh_user(repos).await;
    let prefs = &repos.unit_preferences;

    prefs
        .set_provider_units(user_id, "strava", UnitSystem::Imperial)
        .await
        .unwrap();
    prefs.set_device_locale(user_id, "en-US").await.unwrap();
    prefs
        .set_unit_preference(user_id, UnitPreference::Metric)
        .await
        .unwrap();

    let stored = prefs.get_unit_preferences(user_id).await.unwrap();
    assert_eq!(
        stored,
        StoredUnitPreferences {
            preference: UnitPreference::Metric,
            provider_units: Some(UnitSystem::Imperial),
            provider: Some("strava".to_owned()),
            device_locale: Some("en-US".to_owned()),
        }
    );

    // A later provider read replaces only the provider's setting.
    prefs
        .set_provider_units(user_id, "strava", UnitSystem::Metric)
        .await
        .unwrap();
    let stored = prefs.get_unit_preferences(user_id).await.unwrap();
    assert_eq!(stored.preference, UnitPreference::Metric);
    assert_eq!(stored.provider_units, Some(UnitSystem::Metric));
    assert_eq!(stored.device_locale.as_deref(), Some("en-US"));

    // Scoped by user: the other athlete still has nothing stored.
    assert_eq!(
        prefs.get_unit_preferences(other).await.unwrap(),
        StoredUnitPreferences::default()
    );
}
