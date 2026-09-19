// ABOUTME: Covers the per-user tier override marker against whichever backend DATABASE_URL names
// ABOUTME: Round-trips every column, pins the preserved set_at on upsert, and holds delete to its true/false contract
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `user_tier_overrides` is the marker the billing webhook consults before
//! it lets Stripe move a user's tier. Its three statements are written once
//! and emitted for both backends, differing only in how a uuid reaches the
//! `user_id` and `set_by` columns — native on `PostgreSQL`, `TEXT` on
//! `SQLite`.
//!
//! The repository had no direct test on either backend: the only reads went
//! through `admin_ops` and the webhook, neither of which exercised `set_by`,
//! the preserved `set_at`, or `delete`'s return value. These run on `SQLite`
//! and on `PostgreSQL`: `create_test_db` opens whichever `DATABASE_URL`
//! names, so the same assertions cover both.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use chrono::Utc;
use pierre_core::models::{User, UserTier};
use pierre_database::database::test_utils::create_test_db;
use pierre_database::repositories::UserTierOverride;
use pierre_database::RepositoryRegistry;
use tokio::time::sleep;
use uuid::Uuid;

/// A distinct user per call, so one test's rows cannot satisfy another's
/// assertions; the table's foreign key needs the row to exist.
async fn fresh_user(repos: &RepositoryRegistry) -> Uuid {
    let user = User::new(
        format!("tier-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Tier Override Tester".to_owned()),
    );
    repos.users.create(&user).await.unwrap()
}

#[tokio::test]
async fn upsert_then_get_round_trips_every_column() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(&repos).await;
    let operator_id = fresh_user(&repos).await;

    assert_eq!(
        repos.user_tier_overrides.get(user_id).await.unwrap(),
        None,
        "a user with no marker reads as None so the webhook drives the tier"
    );

    let before = Utc::now();
    let now = Utc::now();
    repos
        .user_tier_overrides
        .upsert(&UserTierOverride {
            user_id,
            tier: UserTier::Professional,
            note: Some("comp account for the launch demo".to_owned()),
            set_by: Some(operator_id),
            set_at: now,
            updated_at: now,
        })
        .await
        .unwrap();

    let stored = repos
        .user_tier_overrides
        .get(user_id)
        .await
        .unwrap()
        .expect("the marker just written must read back");
    assert_eq!(stored.user_id, user_id);
    assert_eq!(stored.tier, UserTier::Professional);
    assert_eq!(
        stored.note.as_deref(),
        Some("comp account for the launch demo")
    );
    assert_eq!(stored.set_by, Some(operator_id));
    assert!(
        stored.set_at >= before && stored.set_at <= Utc::now(),
        "set_at is the write time, not the caller's field"
    );
    assert_eq!(
        stored.set_at, stored.updated_at,
        "a first write stamps both timestamps with the same instant"
    );
}

#[tokio::test]
async fn second_upsert_keeps_set_at_and_replaces_the_rest() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(&repos).await;
    let first_operator = fresh_user(&repos).await;

    let now = Utc::now();
    repos
        .user_tier_overrides
        .upsert(&UserTierOverride {
            user_id,
            tier: UserTier::Starter,
            note: Some("first".to_owned()),
            set_by: Some(first_operator),
            set_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    let first = repos
        .user_tier_overrides
        .get(user_id)
        .await
        .unwrap()
        .unwrap();

    sleep(Duration::from_millis(50)).await;

    let later = Utc::now();
    repos
        .user_tier_overrides
        .upsert(&UserTierOverride {
            user_id,
            tier: UserTier::Enterprise,
            note: None,
            set_by: None,
            set_at: later,
            updated_at: later,
        })
        .await
        .unwrap();
    let second = repos
        .user_tier_overrides
        .get(user_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(second.tier, UserTier::Enterprise);
    assert_eq!(second.note, None, "a NULL note replaces the old one");
    assert_eq!(second.set_by, None, "a NULL set_by replaces the old one");
    assert_eq!(
        second.set_at, first.set_at,
        "set_at is the first-set timestamp and survives the conflict path"
    );
    assert!(
        second.updated_at > first.updated_at,
        "updated_at advances on every write"
    );
}

#[tokio::test]
async fn delete_reports_whether_a_marker_existed() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(&repos).await;

    assert!(
        !repos.user_tier_overrides.delete(user_id).await.unwrap(),
        "deleting a marker that was never set reports false"
    );

    let now = Utc::now();
    repos
        .user_tier_overrides
        .upsert(&UserTierOverride {
            user_id,
            tier: UserTier::Professional,
            note: None,
            set_by: None,
            set_at: now,
            updated_at: now,
        })
        .await
        .unwrap();

    assert!(
        repos.user_tier_overrides.delete(user_id).await.unwrap(),
        "deleting the marker reports true"
    );
    assert_eq!(
        repos.user_tier_overrides.get(user_id).await.unwrap(),
        None,
        "the webhook drives the tier again once the marker is gone"
    );
    assert!(
        !repos.user_tier_overrides.delete(user_id).await.unwrap(),
        "a second delete reports false"
    );
}
