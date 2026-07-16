// ABOUTME: PostgreSQL-specific smoke tests for the user_tool_overrides repository
// ABOUTME: Exercises the native-UUID/BOOLEAN/TIMESTAMPTZ bind paths that SQLite tests cannot catch
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `PostgreSQL` `user_tool_overrides` bind-path tests.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(feature = "postgresql")]

use chrono::Utc;
use pierre_core::models::{CoachingPersona, User, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::Database;
use pierre_database::repositories::UserToolOverride;
use uuid::Uuid;

mod common;

async fn seed_pg_user(db: &Database) -> Uuid {
    let user_id = Uuid::new_v4();
    let user = User {
        id: user_id,
        email: format!("uto-{user_id}@test.local"),
        display_name: Some("Tool Override Test".to_owned()),
        password_hash: "hash_not_verified".to_owned(),
        tier: UserTier::Starter,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: None,
        approved_at: Some(Utc::now()),
        created_at: Utc::now(),
        last_active: Utc::now(),
        strava_token: None,
        fitbit_token: None,
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        default_coach_id: None,
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
        timezone: None,
    };
    db.repositories().users.create(&user).await.unwrap();
    user_id
}

#[tokio::test]
async fn test_pg_user_tool_override_crud_uuid_binds() {
    let isolated = match common::IsolatedPostgresDb::new().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not available: {e}");
            return;
        }
    };
    let db = isolated.get_database().await.unwrap();
    let repos = db.repositories();

    let user_id = seed_pg_user(&db).await;
    let admin_id = seed_pg_user(&db).await;

    // Upsert with a non-NULL set_by exercises both UUID bind positions.
    let now = Utc::now();
    let row = UserToolOverride {
        user_id,
        tool_name: "get_activities".to_owned(),
        is_enabled: false,
        set_by: Some(admin_id),
        reason: Some("pg smoke".to_owned()),
        created_at: now,
        updated_at: now,
    };
    repos.user_tool_overrides.upsert(&row).await.unwrap();

    // Decode path: native UUID, BOOLEAN, TIMESTAMPTZ all round-trip.
    let fetched = repos
        .user_tool_overrides
        .get(user_id, "get_activities")
        .await
        .unwrap()
        .expect("row should exist");
    assert_eq!(fetched.user_id, user_id);
    assert!(!fetched.is_enabled);
    assert_eq!(fetched.set_by, Some(admin_id));
    assert_eq!(fetched.reason.as_deref(), Some("pg smoke"));
    let original_created_at = fetched.created_at;

    // ON CONFLICT path: flip and confirm created_at survives.
    let flipped = UserToolOverride {
        is_enabled: true,
        set_by: None,
        ..row
    };
    repos.user_tool_overrides.upsert(&flipped).await.unwrap();
    let refetched = repos
        .user_tool_overrides
        .get(user_id, "get_activities")
        .await
        .unwrap()
        .unwrap();
    assert!(refetched.is_enabled);
    assert_eq!(refetched.set_by, None, "NULL set_by bind must round-trip");
    assert_eq!(
        refetched.created_at.timestamp(),
        original_created_at.timestamp(),
        "created_at must be preserved across upserts"
    );

    // list_for_user + delete close the loop.
    let listed = repos
        .user_tool_overrides
        .list_for_user(user_id)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].tool_name, "get_activities");

    let removed = repos
        .user_tool_overrides
        .delete(user_id, "get_activities")
        .await
        .unwrap();
    assert!(removed);
    assert!(repos
        .user_tool_overrides
        .list_for_user(user_id)
        .await
        .unwrap()
        .is_empty());
}
