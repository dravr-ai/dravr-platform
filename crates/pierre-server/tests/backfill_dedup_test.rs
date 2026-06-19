// ABOUTME: Integration tests for the durable cross-replica backfill-push dedup claim
// ABOUTME: Asserts claim_backfill_push is single-claim, window/provider/tenant-scoped, and tenant-isolated
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate
)]

#[cfg(feature = "postgresql")]
use pierre_config::environment::PostgresPoolConfig;
use pierre_core::models::TenantId;
use pierre_database::backends::factory::Database;
use pierre_database::DatabaseProvider;
use uuid::Uuid;

/// Create an in-memory `SQLite` database with all migrations applied — this
/// exercises the new `20260619000002_backfill_push_log` migration on every run.
async fn create_test_db() -> Database {
    let encryption_key = b"test_encryption_key_32_bytes_long".to_vec();

    #[cfg(feature = "postgresql")]
    let db = Database::new(
        "sqlite::memory:",
        encryption_key,
        &PostgresPoolConfig::default(),
    )
    .await
    .expect("Failed to create test database");

    #[cfg(not(feature = "postgresql"))]
    let db = Database::new("sqlite::memory:", encryption_key)
        .await
        .expect("Failed to create test database");

    db.migrate().await.expect("Failed to run migrations");
    db
}

/// The first claim for a `(tenant, user, provider, after_ts)` window wins
/// (`true`); the identical second claim loses (`false`). This is the
/// exactly-once guarantee the completion push relies on.
#[tokio::test]
async fn claim_is_single_use_per_window() {
    let db = create_test_db().await;
    let repos = db.repositories();
    let tenant_id = TenantId::new();
    let user_id = Uuid::new_v4().to_string();

    let first = repos
        .messaging
        .claim_backfill_push(tenant_id, &user_id, "strava", 1_700_000_000)
        .await
        .unwrap();
    assert!(first, "first claim for a fresh window must win");

    let second = repos
        .messaging
        .claim_backfill_push(tenant_id, &user_id, "strava", 1_700_000_000)
        .await
        .unwrap();
    assert!(
        !second,
        "a repeat claim for the same window must lose (already sent)"
    );
}

/// A different `after_ts` is a different window and claims independently — so a
/// later, deeper backfill still pushes once.
#[tokio::test]
async fn different_after_ts_claims_independently() {
    let db = create_test_db().await;
    let repos = db.repositories();
    let tenant_id = TenantId::new();
    let user_id = Uuid::new_v4().to_string();

    assert!(
        repos
            .messaging
            .claim_backfill_push(tenant_id, &user_id, "strava", 1_700_000_000)
            .await
            .unwrap(),
        "first window claims"
    );
    assert!(
        repos
            .messaging
            .claim_backfill_push(tenant_id, &user_id, "strava", 1_600_000_000)
            .await
            .unwrap(),
        "a different after_ts is a different window and claims independently"
    );
}

/// A different provider claims independently — backfilling Strava then Garmin
/// for the same user/window pushes for each.
#[tokio::test]
async fn different_provider_claims_independently() {
    let db = create_test_db().await;
    let repos = db.repositories();
    let tenant_id = TenantId::new();
    let user_id = Uuid::new_v4().to_string();

    assert!(
        repos
            .messaging
            .claim_backfill_push(tenant_id, &user_id, "strava", 1_700_000_000)
            .await
            .unwrap(),
        "strava window claims"
    );
    assert!(
        repos
            .messaging
            .claim_backfill_push(tenant_id, &user_id, "garmin", 1_700_000_000)
            .await
            .unwrap(),
        "a different provider is a different window and claims independently"
    );
}

/// Tenant isolation: the same `(user, provider, after_ts)` under a different
/// tenant claims independently — `tenant_id` is part of the claim key, so one
/// tenant's claim never suppresses another's.
#[tokio::test]
async fn claim_is_tenant_isolated() {
    let db = create_test_db().await;
    let repos = db.repositories();
    let tenant_a = TenantId::new();
    let tenant_b = TenantId::new();
    let user_id = Uuid::new_v4().to_string();

    assert!(
        repos
            .messaging
            .claim_backfill_push(tenant_a, &user_id, "strava", 1_700_000_000)
            .await
            .unwrap(),
        "tenant A claims"
    );
    assert!(
        repos
            .messaging
            .claim_backfill_push(tenant_b, &user_id, "strava", 1_700_000_000)
            .await
            .unwrap(),
        "tenant B claims independently — claim is tenant-scoped"
    );
    // And tenant A re-claiming the same window still loses, proving the prior
    // tenant-B claim did not reset tenant A's row.
    assert!(
        !repos
            .messaging
            .claim_backfill_push(tenant_a, &user_id, "strava", 1_700_000_000)
            .await
            .unwrap(),
        "tenant A's window stays claimed after tenant B claims"
    );
}
