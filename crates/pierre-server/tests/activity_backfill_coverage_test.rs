// ABOUTME: Round-trip + overwrite + isolation tests for the activity_backfill_coverage repo
// ABOUTME: Backs the historical gate's self-heal — coverage deepens, never loses feed-end
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

#[cfg(feature = "postgresql")]
use pierre_config::environment::PostgresPoolConfig;
use pierre_core::models::TenantId;
use pierre_database::backends::factory::Database;
use pierre_database::repositories::BackfillCoverage;
use pierre_database::DatabaseProvider;
use uuid::Uuid;

/// In-memory `SQLite` DB with all migrations applied (creates
/// `activity_backfill_coverage`).
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

/// The coverage record round-trips, a deeper backfill overwrites it (coverage
/// only deepens), and reads are scoped to `(tenant, user, provider)`.
#[tokio::test]
async fn backfill_coverage_round_trips_overwrites_and_isolates() {
    let db = create_test_db().await;
    let repos = db.repositories();
    let cache = &repos.activity_cache;
    let user = Uuid::new_v4();
    let tenant = TenantId::new();

    // Nothing recorded yet.
    assert!(cache
        .get_backfill_coverage(user, &tenant, "strava")
        .await
        .unwrap()
        .is_none());

    // A shallow, non-feed-end backfill (reached only mid-2022).
    cache
        .upsert_backfill_coverage(
            user,
            &tenant,
            "strava",
            BackfillCoverage {
                oldest_reached_ts: 1_657_000_000,
                hit_feed_end: false,
            },
        )
        .await
        .unwrap();
    let got = cache
        .get_backfill_coverage(user, &tenant, "strava")
        .await
        .unwrap()
        .expect("coverage was just written");
    assert_eq!(got.oldest_reached_ts, 1_657_000_000);
    assert!(!got.hit_feed_end);

    // A deeper backfill that exhausts the feed overwrites it.
    cache
        .upsert_backfill_coverage(
            user,
            &tenant,
            "strava",
            BackfillCoverage {
                oldest_reached_ts: 1_640_000_000,
                hit_feed_end: true,
            },
        )
        .await
        .unwrap();
    let got2 = cache
        .get_backfill_coverage(user, &tenant, "strava")
        .await
        .unwrap()
        .expect("coverage still present after overwrite");
    assert_eq!(got2.oldest_reached_ts, 1_640_000_000);
    assert!(got2.hit_feed_end);

    // Isolation: a different tenant or provider has no coverage.
    assert!(cache
        .get_backfill_coverage(user, &TenantId::new(), "strava")
        .await
        .unwrap()
        .is_none());
    assert!(cache
        .get_backfill_coverage(user, &tenant, "garmin")
        .await
        .unwrap()
        .is_none());
}
