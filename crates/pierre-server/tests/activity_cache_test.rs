// ABOUTME: Integration tests for the provider-agnostic activity cache repository
// ABOUTME: Exercises upsert (idempotent), range + provider-filtered reads, freshness, and retention pruning
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Verifies the `ActivityCacheRepository` contract that backs
//! stale-while-revalidate reads on the chat path:
//!  1. `upsert_activities` persists a batch and is idempotent on re-fetch
//!     (keyed on `(user, tenant, provider, activity_id)`).
//!  2. `get_cached_activities` returns rows in `[start, end]` newest-first and
//!     honours the optional provider filter.
//!  3. `latest_activity_sync` reports a timestamp once data is cached.
//!  4. `prune_activities_before` enforces retention.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use chrono::{Duration, Utc};
use pierre_core::models::{Activity, ActivityBuilder, SportType, TenantId};

fn activity(id: &str, provider: &str, age_days: i64) -> Activity {
    let start = Utc::now() - Duration::days(age_days);
    ActivityBuilder::new(
        id.to_owned(),
        format!("activity {id}"),
        SportType::Run,
        start,
        3_600,
        provider.to_owned(),
    )
    .distance_meters(10_000.0)
    .build()
}

#[tokio::test]
async fn activity_cache_upsert_read_filter_prune_round_trip() {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let tenant_id = TenantId::new();
    let repo = &repos.activity_cache;

    let window_start = Utc::now() - Duration::days(90);
    let now = Utc::now();

    // 1. Upsert two Strava activities.
    let acts = vec![activity("s1", "strava", 1), activity("s2", "strava", 3)];
    let written = repo
        .upsert_activities(user_id, &tenant_id, "strava", &acts)
        .await
        .unwrap();
    assert_eq!(written, 2);

    // 2. Range read returns both, newest first.
    let got = repo
        .get_cached_activities(user_id, &tenant_id, None, window_start, now, 100)
        .await
        .unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].id(), "s1"); // 1 day old is newer than 3 days old
    assert_eq!(got[1].id(), "s2");

    // 3. Idempotent re-fetch: same activity_ids overwrite, count stays 2.
    repo.upsert_activities(user_id, &tenant_id, "strava", &acts)
        .await
        .unwrap();
    let got = repo
        .get_cached_activities(user_id, &tenant_id, None, window_start, now, 100)
        .await
        .unwrap();
    assert_eq!(got.len(), 2);

    // 4. Provider filter isolates rows.
    let strava = repo
        .get_cached_activities(user_id, &tenant_id, Some("strava"), window_start, now, 100)
        .await
        .unwrap();
    assert_eq!(strava.len(), 2);
    let garmin = repo
        .get_cached_activities(user_id, &tenant_id, Some("garmin"), window_start, now, 100)
        .await
        .unwrap();
    assert_eq!(garmin.len(), 0);

    // 5. Freshness signal present for the written provider, absent otherwise.
    assert!(repo
        .latest_activity_sync(user_id, &tenant_id, "strava")
        .await
        .unwrap()
        .is_some());
    assert!(repo
        .latest_activity_sync(user_id, &tenant_id, "garmin")
        .await
        .unwrap()
        .is_none());

    // 6. A second provider's rows coexist and merge into the unfiltered read.
    let garmin_acts = vec![activity("g1", "garmin", 2)];
    repo.upsert_activities(user_id, &tenant_id, "garmin", &garmin_acts)
        .await
        .unwrap();
    let all = repo
        .get_cached_activities(user_id, &tenant_id, None, window_start, now, 100)
        .await
        .unwrap();
    assert_eq!(all.len(), 3);

    // 7. Retention prune removes rows older than the cutoff (drops s2 @3d).
    let cutoff = Utc::now() - Duration::days(2) - Duration::hours(12);
    let removed = repo
        .prune_activities_before(user_id, &tenant_id, cutoff)
        .await
        .unwrap();
    assert_eq!(removed, 1);
    let remaining = repo
        .get_cached_activities(user_id, &tenant_id, None, window_start, now, 100)
        .await
        .unwrap();
    assert_eq!(remaining.len(), 2);
}

#[tokio::test]
async fn activity_cache_is_tenant_isolated() {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let tenant_a = TenantId::new();
    let tenant_b = TenantId::new();

    let acts = vec![activity("a1", "strava", 1)];
    repos
        .activity_cache
        .upsert_activities(user_id, &tenant_a, "strava", &acts)
        .await
        .unwrap();

    let window_start = Utc::now() - Duration::days(90);
    let now = Utc::now();

    let in_a = repos
        .activity_cache
        .get_cached_activities(user_id, &tenant_a, None, window_start, now, 100)
        .await
        .unwrap();
    assert_eq!(in_a.len(), 1);

    // Same user, different tenant must not see tenant A's cached activities.
    let in_b = repos
        .activity_cache
        .get_cached_activities(user_id, &tenant_b, None, window_start, now, 100)
        .await
        .unwrap();
    assert_eq!(in_b.len(), 0);
}
