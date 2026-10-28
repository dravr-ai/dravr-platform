// ABOUTME: Round-trip + overwrite + isolation tests for the activity_backfill_coverage repo
// ABOUTME: Backs the historical gate's self-heal — coverage deepens, never loses feed-end
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::db_fixtures;
use db_fixtures::create_test_db;

use chrono::{Duration, Utc};
use pierre_core::constants::provider_capture::{
    BASELINE_CAPTURE_VERSION, SCIOTTE_STRAVA_CAPTURE_VERSION,
};
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
                capture_version: BASELINE_CAPTURE_VERSION,
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
                capture_version: BASELINE_CAPTURE_VERSION,
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

/// A prune of one provider raises that provider's coverage floor it just
/// falsified, and leaves other providers' claims (whose rows it never
/// touched), shallower claims and other athletes alone.
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
                    capture_version: BASELINE_CAPTURE_VERSION,
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
                capture_version: BASELINE_CAPTURE_VERSION,
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
                capture_version: BASELINE_CAPTURE_VERSION,
            },
        )
        .await
        .unwrap();

    // A Strava prune raises Strava's claim and only Strava's: Garmin's rows
    // were never pruned, so its claim is still true.
    let clamp =
        |provider: &'static str| cache.clamp_backfill_coverage(user, &tenant, provider, cutoff);
    assert_eq!(
        clamp("strava").await.unwrap(),
        1,
        "Strava's 2022 claim is raised"
    );
    let garmin = cache
        .get_backfill_coverage(user, &tenant, "garmin")
        .await
        .unwrap()
        .expect("garmin coverage");
    assert_eq!(
        garmin.oldest_reached_ts, deep,
        "bug: a Strava prune must not move Garmin's claim"
    );
    assert!(garmin.hit_feed_end);
    assert_eq!(
        clamp("garmin").await.unwrap(),
        1,
        "Garmin's own prune raises it"
    );
    assert_eq!(
        clamp("whoop").await.unwrap(),
        0,
        "a shallower claim never moves"
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

/// The capture version round-trips, a re-capture replaces the older record's
/// floor instead of inheriting it, and the retention clamp leaves it alone.
///
/// The version names the capture that WROTE the rows. A re-capture that only
/// went sixteen weeks deep must not come out claiming the 2022 depth the old
/// capture reached — those rows were never re-read. And a prune raising the
/// floor re-reads nothing at all, so it must not promote the record either.
#[tokio::test]
async fn the_capture_version_follows_the_backfill_not_the_clamp() {
    let db = create_test_db().await;
    let repos = db.repositories();
    let cache = &repos.activity_cache;
    let user = Uuid::new_v4();
    let tenant = TenantId::generate();

    let deep = 1_640_995_200; // Jan 1 2022
    cache
        .upsert_backfill_coverage(
            user,
            &tenant,
            "sciotte",
            BackfillCoverage {
                oldest_reached_ts: deep,
                hit_feed_end: true,
                capture_version: BASELINE_CAPTURE_VERSION,
            },
        )
        .await
        .unwrap();
    let old = cache
        .get_backfill_coverage(user, &tenant, "sciotte")
        .await
        .unwrap()
        .expect("coverage was just written");
    assert_eq!(old.capture_version, BASELINE_CAPTURE_VERSION);

    // The clamp raises the floor and re-reads nothing: the version stays put.
    let cutoff = Utc::now() - Duration::days(180);
    assert_eq!(
        cache
            .clamp_backfill_coverage(user, &tenant, "sciotte", cutoff)
            .await
            .unwrap(),
        1
    );
    let clamped = cache
        .get_backfill_coverage(user, &tenant, "sciotte")
        .await
        .unwrap()
        .expect("coverage survives the clamp");
    assert_eq!(clamped.oldest_reached_ts, cutoff.timestamp());
    assert_eq!(
        clamped.capture_version, BASELINE_CAPTURE_VERSION,
        "a prune re-captured nothing, so it must not vouch for the rows as current"
    );

    // A sixteen-week re-capture through the corrected scraper.
    let sixteen_weeks = (Utc::now() - Duration::weeks(16)).timestamp();
    cache
        .upsert_backfill_coverage(
            user,
            &tenant,
            "sciotte",
            BackfillCoverage {
                oldest_reached_ts: sixteen_weeks,
                hit_feed_end: false,
                capture_version: SCIOTTE_STRAVA_CAPTURE_VERSION,
            },
        )
        .await
        .unwrap();
    let recaptured = cache
        .get_backfill_coverage(user, &tenant, "sciotte")
        .await
        .unwrap()
        .expect("coverage present after the re-capture");
    assert_eq!(recaptured.capture_version, SCIOTTE_STRAVA_CAPTURE_VERSION);
    assert_eq!(
        recaptured.oldest_reached_ts, sixteen_weeks,
        "the new record vouches only for the depth the new capture re-read"
    );
    assert!(!recaptured.hit_feed_end);
}
