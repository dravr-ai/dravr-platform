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

use chrono::{DateTime, Duration, Utc};
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

/// The read's `updated_at` is the concurrency token: it reaches a super admin as
/// RFC 3339 text and comes back, so the row must match it to the instant. That
/// is what `$n` binds a `DateTime` round-trip proves, on each backend.
#[tokio::test]
async fn a_compare_and_set_lands_only_on_the_row_it_read() {
    let db = create_test_db().await.unwrap();
    let key = format!("test_setting_{}", Uuid::new_v4().simple());

    // No row yet: the token "absent" creates it, a timestamp matches nothing.
    assert_eq!(
        db.compare_and_set_system_setting(&key, "ghost", Some(Utc::now()))
            .await
            .unwrap(),
        None
    );
    assert!(db.get_system_setting(&key).await.unwrap().is_none());
    let created = db
        .compare_and_set_system_setting(&key, "first", None)
        .await
        .unwrap()
        .expect("an absent row is created");

    // The stamp the write returned is the stamp the row reads back with.
    let row = db.get_system_setting(&key).await.unwrap().unwrap();
    assert_eq!(row.value, "first");
    assert_eq!(row.updated_at, created);

    // "Absent" no longer holds, so a second creator cannot overwrite.
    assert_eq!(
        db.compare_and_set_system_setting(&key, "second creator", None)
            .await
            .unwrap(),
        None
    );

    // The token survives the wire form a super admin's client echoes.
    let token = DateTime::parse_from_rfc3339(&row.updated_at.to_rfc3339())
        .unwrap()
        .with_timezone(&Utc);
    let moved = db
        .compare_and_set_system_setting(&key, "second", Some(token))
        .await
        .unwrap()
        .expect("the row is as the token read it");
    assert_ne!(moved, token, "a landed write moves the token");

    // The spent token is refused and the row keeps the winner's value.
    assert_eq!(
        db.compare_and_set_system_setting(&key, "stale", Some(token))
            .await
            .unwrap(),
        None
    );
    let row = db.get_system_setting(&key).await.unwrap().unwrap();
    assert_eq!(row.value, "second");
    assert_eq!(row.updated_at, moved);
}

/// A row the plain upsert wrote carries a clock stamp, not a truncated one. The
/// next compare-and-set must still match it.
#[tokio::test]
async fn a_compare_and_set_matches_a_row_the_plain_upsert_wrote() {
    let db = create_test_db().await.unwrap();
    let key = format!("test_setting_{}", Uuid::new_v4().simple());

    db.set_system_setting(&key, "first").await.unwrap();
    let row = db.get_system_setting(&key).await.unwrap().unwrap();
    let token = DateTime::parse_from_rfc3339(&row.updated_at.to_rfc3339())
        .unwrap()
        .with_timezone(&Utc);

    assert!(db
        .compare_and_set_system_setting(&key, "second", Some(token))
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        db.get_system_setting(&key).await.unwrap().unwrap().value,
        "second"
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
