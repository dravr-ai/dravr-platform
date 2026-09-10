// ABOUTME: Round-trip + overwrite + isolation tests for the activity_backfill_coverage repo
// ABOUTME: Backs the historical gate's self-heal — coverage deepens, never loses feed-end
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

#[path = "helpers/db_fixtures.rs"]
mod db_fixtures;
use db_fixtures::create_test_db;

use chrono::{Duration, Utc};
use pierre_core::models::TenantId;
use pierre_database::repositories::BackfillCoverage;
use uuid::Uuid;

/// The coverage record round-trips, a deeper backfill overwrites it (coverage
/// only deepens), and reads are scoped to `(tenant, user, provider)`.
#[tokio::test]
async fn backfill_coverage_round_trips_overwrites_and_isolates() {
    let db = create_test_db().await;
    let repos = db.repositories();
    let cache = &repos.activity_cache;
    let user = Uuid::new_v4();
    let tenant = TenantId::generate();

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
        .get_backfill_coverage(user, &TenantId::generate(), "strava")
        .await
        .unwrap()
        .is_none());
    assert!(cache
        .get_backfill_coverage(user, &tenant, "garmin")
        .await
        .unwrap()
        .is_none());
}

/// A prune raises every coverage floor it just falsified, across providers, and
/// leaves shallower claims and other athletes alone.
///
/// Coverage records what a backfill FETCHED; the prune decides what is still
/// STORED. Left unclamped, a record goes on naming a depth whose rows are gone
/// and the historical gate serves that cache as a complete window without
/// calling a provider (registre#408).
#[tokio::test]
async fn a_prune_raises_the_coverage_floors_it_falsified() {
    let db = create_test_db().await;
    let repos = db.repositories();
    let cache = &repos.activity_cache;
    let user = Uuid::new_v4();
    let other_user = Uuid::new_v4();
    let tenant = TenantId::generate();

    let deep = 1_640_995_200; // Jan 1 2022
    let cutoff = Utc::now() - Duration::days(180);

    // Two providers claiming 2022 depth, one of them on feed-end.
    for (provider, hit_feed_end) in [("strava", false), ("garmin", true)] {
        cache
            .upsert_backfill_coverage(
                user,
                &tenant,
                provider,
                BackfillCoverage {
                    oldest_reached_ts: deep,
                    hit_feed_end,
                },
            )
            .await
            .unwrap();
    }
    // A claim already shallower than the cutoff must not move.
    let shallow = Utc::now().timestamp();
    cache
        .upsert_backfill_coverage(
            user,
            &tenant,
            "whoop",
            BackfillCoverage {
                oldest_reached_ts: shallow,
                hit_feed_end: false,
            },
        )
        .await
        .unwrap();
    // A different athlete's deep claim is untouched.
    cache
        .upsert_backfill_coverage(
            other_user,
            &tenant,
            "strava",
            BackfillCoverage {
                oldest_reached_ts: deep,
                hit_feed_end: true,
            },
        )
        .await
        .unwrap();

    let clamped = cache
        .clamp_backfill_coverage(user, &tenant, cutoff)
        .await
        .unwrap();
    assert_eq!(
        clamped, 2,
        "both 2022 claims are raised, the recent one is not"
    );

    for provider in ["strava", "garmin"] {
        let c = cache
            .get_backfill_coverage(user, &tenant, provider)
            .await
            .unwrap()
            .expect("coverage row survives the clamp");
        assert_eq!(
            c.oldest_reached_ts,
            cutoff.timestamp(),
            "{provider} must claim only what the prune left"
        );
        assert!(
            !c.hit_feed_end,
            "{provider}: feed-end says the provider has nothing older, not that the cache kept it"
        );
    }

    let untouched = cache
        .get_backfill_coverage(user, &tenant, "whoop")
        .await
        .unwrap()
        .expect("shallow row survives");
    assert_eq!(untouched.oldest_reached_ts, shallow);

    let peer = cache
        .get_backfill_coverage(other_user, &tenant, "strava")
        .await
        .unwrap()
        .expect("another athlete's row survives");
    assert_eq!(
        peer.oldest_reached_ts, deep,
        "the clamp is scoped to one athlete"
    );
    assert!(peer.hit_feed_end);
}
