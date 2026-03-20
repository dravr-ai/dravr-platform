// ABOUTME: Tests for the usage counter tracking database module
// ABOUTME: Validates increment upsert, get with zero default, and old counter cleanup
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Test files: allow missing_docs (rustc lint) and unwrap (valid in tests per CLAUDE.md guidelines)
#![allow(missing_docs, clippy::unwrap_used)]

use pierre_database::database::test_utils::create_test_db;

#[tokio::test]
async fn test_increment_counter_creates_new() {
    let db = create_test_db().await.unwrap();

    let record = db
        .repositories()
        .usage_counters
        .increment_counter("tenant-1", "user-1", "messages", "2026-02-17", 1)
        .await
        .unwrap();

    assert_eq!(record.tenant_id, "tenant-1");
    assert_eq!(record.user_id, "user-1");
    assert_eq!(record.counter_key, "messages");
    assert_eq!(record.period, "2026-02-17");
    assert_eq!(record.value, 1);
}

#[tokio::test]
async fn test_increment_counter_upserts_existing() {
    let db = create_test_db().await.unwrap();

    db.repositories()
        .usage_counters
        .increment_counter("tenant-1", "user-1", "messages", "2026-02-17", 5)
        .await
        .unwrap();

    let record = db
        .repositories()
        .usage_counters
        .increment_counter("tenant-1", "user-1", "messages", "2026-02-17", 3)
        .await
        .unwrap();

    assert_eq!(record.value, 8);
}

#[tokio::test]
async fn test_get_counter_returns_zero_when_missing() {
    let db = create_test_db().await.unwrap();

    let record = db
        .repositories()
        .usage_counters
        .get_counter("tenant-1", "user-1", "messages", "2026-02-17")
        .await
        .unwrap();

    assert_eq!(record.value, 0);
    assert_eq!(record.tenant_id, "tenant-1");
    assert_eq!(record.counter_key, "messages");
}

#[tokio::test]
async fn test_get_counter_returns_current_value() {
    let db = create_test_db().await.unwrap();

    db.repositories()
        .usage_counters
        .increment_counter("tenant-1", "user-1", "tool_calls", "2026-W08", 10)
        .await
        .unwrap();

    let record = db
        .repositories()
        .usage_counters
        .get_counter("tenant-1", "user-1", "tool_calls", "2026-W08")
        .await
        .unwrap();

    assert_eq!(record.value, 10);
}

#[tokio::test]
async fn test_delete_old_counters() {
    let db = create_test_db().await.unwrap();

    // Create counters across multiple periods
    db.repositories()
        .usage_counters
        .increment_counter("tenant-1", "user-1", "messages", "2026-02-10", 5)
        .await
        .unwrap();
    db.repositories()
        .usage_counters
        .increment_counter("tenant-1", "user-1", "messages", "2026-02-15", 10)
        .await
        .unwrap();
    db.repositories()
        .usage_counters
        .increment_counter("tenant-1", "user-1", "messages", "2026-02-17", 3)
        .await
        .unwrap();

    // Delete counters older than 2026-02-16
    let deleted = db
        .repositories()
        .usage_counters
        .delete_old_counters("2026-02-16")
        .await
        .unwrap();
    assert_eq!(deleted, 2);

    // Verify the remaining counter
    let remaining = db
        .repositories()
        .usage_counters
        .get_counter("tenant-1", "user-1", "messages", "2026-02-17")
        .await
        .unwrap();
    assert_eq!(remaining.value, 3);

    // Verify old counters are gone
    let gone = db
        .repositories()
        .usage_counters
        .get_counter("tenant-1", "user-1", "messages", "2026-02-10")
        .await
        .unwrap();
    assert_eq!(gone.value, 0);
}

#[tokio::test]
async fn test_counters_isolated_by_tenant() {
    let db = create_test_db().await.unwrap();

    db.repositories()
        .usage_counters
        .increment_counter("tenant-a", "user-1", "messages", "2026-02-17", 10)
        .await
        .unwrap();
    db.repositories()
        .usage_counters
        .increment_counter("tenant-b", "user-1", "messages", "2026-02-17", 20)
        .await
        .unwrap();

    let a = db
        .repositories()
        .usage_counters
        .get_counter("tenant-a", "user-1", "messages", "2026-02-17")
        .await
        .unwrap();
    let b = db
        .repositories()
        .usage_counters
        .get_counter("tenant-b", "user-1", "messages", "2026-02-17")
        .await
        .unwrap();

    assert_eq!(a.value, 10);
    assert_eq!(b.value, 20);
}
