// ABOUTME: SQLite integration tests for UserOnboardingRepository — durable per-user onboarding step state
// ABOUTME: Runs on the default sqlite::memory: harness (no Postgres needed); parity vs PG lives in database_parity_test
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Repository-level tests for `user_onboarding` on `SQLite`: upsert-in-place on the
//! `(user_id, step_id)` key, nullable `chosen_channel` round-trip, and per-user
//! scoping.

mod common;

use uuid::Uuid;

#[tokio::test]
async fn user_onboarding_step_roundtrip_and_upsert() {
    common::init_server_config();
    let database = common::create_test_database()
        .await
        .expect("sqlite test db");
    let repos = database.repositories();
    let (user_id, _user) = common::create_test_user(&database)
        .await
        .expect("test user");
    let uid = user_id.to_string();

    // A plain step, plus the messaging-channel step carrying a chosen_channel.
    repos
        .user_onboarding
        .set_onboarding_step(&uid, "profile_type", "complete", None, Some("t1"))
        .await
        .expect("set profile_type");
    repos
        .user_onboarding
        .set_onboarding_step(
            &uid,
            "messaging_channel",
            "complete",
            Some("telegram"),
            Some("t1"),
        )
        .await
        .expect("set messaging_channel");

    // Re-mark profile_type: the upsert must overwrite in place, not duplicate.
    repos
        .user_onboarding
        .set_onboarding_step(&uid, "profile_type", "skipped", None, None)
        .await
        .expect("re-set profile_type");

    let mut steps = repos
        .user_onboarding
        .get_onboarding_steps(&uid)
        .await
        .expect("get steps");
    steps.sort_by(|a, b| a.step_id.cmp(&b.step_id));

    assert_eq!(
        steps.len(),
        2,
        "upsert overwrote profile_type rather than inserting"
    );

    let messaging = steps
        .iter()
        .find(|s| s.step_id == "messaging_channel")
        .expect("messaging_channel row");
    assert_eq!(messaging.status, "complete");
    assert_eq!(messaging.chosen_channel.as_deref(), Some("telegram"));

    let profile = steps
        .iter()
        .find(|s| s.step_id == "profile_type")
        .expect("profile_type row");
    assert_eq!(profile.status, "skipped", "upsert overwrote the status");
    assert_eq!(
        profile.chosen_channel, None,
        "NULL chosen_channel reads back as None"
    );
}

#[tokio::test]
async fn user_onboarding_steps_are_scoped_per_user() {
    common::init_server_config();
    let database = common::create_test_database()
        .await
        .expect("sqlite test db");
    let repos = database.repositories();
    let (user_id, _u1) = common::create_test_user(&database).await.expect("user a");

    repos
        .user_onboarding
        .set_onboarding_step(&user_id.to_string(), "profile_type", "complete", None, None)
        .await
        .expect("set for user a");

    // A different user_id (never inserted — no FK, so it need not exist) sees none
    // of user a's rows: reads are scoped by user_id.
    let other_id = Uuid::new_v4().to_string();
    let other = repos
        .user_onboarding
        .get_onboarding_steps(&other_id)
        .await
        .expect("get for a different user");
    assert!(
        other.is_empty(),
        "a different user must not see user a's steps"
    );
}
