// ABOUTME: Pins the system-settings store, written once for both backends, against whichever backend DATABASE_URL names
// ABOUTME: A setting round-trips its value and timestamp, an overwrite moves both, and an unknown key reads as absent

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The key/value table behind auto-approval, the guardian and harness
//! configs and the email-verification limits used to carry its SQL twice,
//! once per backend, with the timestamp bound as text on one side and by the
//! server clock on the other. One body now serves both; these assertions run
//! on `SQLite` and on `PostgreSQL` alike, since `create_test_db` opens
//! whichever `DATABASE_URL` names.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration as StdDuration;

use chrono::{Duration, Utc};
use pierre_test_support::db::create_test_db;
use tokio::time::sleep;
use uuid::Uuid;

#[tokio::test]
async fn a_setting_round_trips_and_an_overwrite_moves_its_timestamp() {
    let db = create_test_db().await.unwrap();
    let key = format!("test_setting_{}", Uuid::new_v4().simple());

    assert!(db.get_system_setting(&key).await.unwrap().is_none());

    let before = Utc::now() - Duration::seconds(1);
    db.set_system_setting(&key, "first").await.unwrap();
    let first = db
        .get_system_setting(&key)
        .await
        .unwrap()
        .expect("the setting reads back");
    assert_eq!(first.key, key);
    assert_eq!(first.value, "first");
    assert_eq!(first.description, None);
    assert!(
        first.updated_at >= before && first.updated_at <= Utc::now(),
        "updated_at is the write's instant, got {}",
        first.updated_at
    );

    sleep(StdDuration::from_millis(1100)).await;
    db.set_system_setting(&key, "second").await.unwrap();
    let second = db
        .get_system_setting(&key)
        .await
        .unwrap()
        .expect("the overwritten setting reads back");
    assert_eq!(second.value, "second");
    assert!(
        second.updated_at > first.updated_at,
        "an overwrite moves updated_at: {} then {}",
        first.updated_at,
        second.updated_at
    );
}

#[tokio::test]
async fn auto_approval_reads_back_as_written() {
    let db = create_test_db().await.unwrap();

    db.set_auto_approval_enabled(true).await.unwrap();
    assert_eq!(db.is_auto_approval_enabled().await.unwrap(), Some(true));
    db.set_auto_approval_enabled(false).await.unwrap();
    assert_eq!(db.is_auto_approval_enabled().await.unwrap(), Some(false));
}
