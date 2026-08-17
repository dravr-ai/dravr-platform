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

use chrono::{DateTime, Duration, TimeZone, Utc};
use pierre_core::models::{Activity, ActivityBuilder, SportType, TenantId};
use pierre_database::backends::factory::Database;
use sqlx::Row;

fn activity(id: &str, provider: &str, age_days: i64) -> Activity {
    let start = Utc::now() - Duration::days(age_days);
    activity_at(id, provider, start)
}

/// Build an activity with an explicit `start_date`, so tests can place rows at a
/// fixed point in a historical window (e.g. the 2022 season) independent of the
/// current time.
fn activity_at(id: &str, provider: &str, start: DateTime<Utc>) -> Activity {
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
    let tenant_id = TenantId::generate();
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
    let tenant_a = TenantId::generate();
    let tenant_b = TenantId::generate();

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

/// The returned count must be the NET DISTINCT rows persisted (deduped by
/// `activity_id`), never the raw input length. A provider feed can repeat an
/// `activity_id` within one batch; each ON CONFLICT upsert overwrites the same
/// row, so the input length overstates the distinct rows stored. This guards the
/// honest count that feeds the backfill completion notice.
#[tokio::test]
async fn activity_cache_upsert_count_is_distinct_not_input_length() {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let tenant_id = TenantId::generate();
    let repo = &repos.activity_cache;

    // Five input rows but only THREE distinct activity_ids — "d1" appears three
    // times (the provider feed repeated it within the batch).
    let acts = vec![
        activity("d1", "strava", 1),
        activity("d2", "strava", 2),
        activity("d1", "strava", 1),
        activity("d3", "strava", 3),
        activity("d1", "strava", 1),
    ];
    assert_eq!(acts.len(), 5, "fixture should have 5 raw input rows");

    let persisted = repo
        .upsert_activities(user_id, &tenant_id, "strava", &acts)
        .await
        .unwrap();
    // Honest count: 3 distinct rows, NOT 5.
    assert_eq!(
        persisted, 3,
        "upsert must report net distinct rows (3), not raw input length (5)"
    );

    // And the durable table holds exactly those 3 distinct rows.
    let window_start = Utc::now() - Duration::days(90);
    let now = Utc::now();
    let stored = repo
        .get_cached_activities(user_id, &tenant_id, Some("strava"), window_start, now, 100)
        .await
        .unwrap();
    assert_eq!(stored.len(), 3, "only 3 distinct rows should be stored");
}

/// Number of distinct historical rows seeded by the determinism test.
const K: usize = 50;

/// A historical window must read DETERMINISTICALLY: reading the same `[after,
/// before]` window twice returns the identical complete set of all K seeded
/// rows, regardless of how many times it is read or what display limit a caller
/// would later apply. This is the durable-cache invariant the `get_activities`
/// historical gate now relies on (single deterministic read for both the
/// coverage decision and the served list, instead of two diverging reads).
#[tokio::test]
async fn activity_cache_historical_window_read_is_deterministic_and_complete() {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let tenant_id = TenantId::generate();
    let repo = &repos.activity_cache;

    // Seed K=50 distinct rows spread across the 2022 season [2022-01-01,
    // 2023-01-01). One per week so they comfortably fit the year window.
    let mut acts = Vec::with_capacity(K);
    for i in 0..K {
        let week = i64::try_from(i).unwrap();
        let start = Utc.with_ymd_and_hms(2022, 1, 1, 12, 0, 0).unwrap() + Duration::weeks(week);
        acts.push(activity_at(&format!("h{i}"), "strava", start));
    }
    let persisted = repo
        .upsert_activities(user_id, &tenant_id, "strava", &acts)
        .await
        .unwrap();
    assert_eq!(
        persisted,
        u64::try_from(K).unwrap(),
        "all 50 distinct rows persisted"
    );

    let after = Utc.with_ymd_and_hms(2022, 1, 1, 0, 0, 0).unwrap();
    let before = Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap();
    // A high read limit (mirrors the gate's window read limit) so the full
    // window comes back rather than a small display limit truncating it.
    let read_limit = 2_000;

    // Read the same window TWICE. Both reads must return the identical complete
    // set of all K rows — no divergence, no partial result.
    let first = repo
        .get_cached_activities(
            user_id,
            &tenant_id,
            Some("strava"),
            after,
            before,
            read_limit,
        )
        .await
        .unwrap();
    let second = repo
        .get_cached_activities(
            user_id,
            &tenant_id,
            Some("strava"),
            after,
            before,
            read_limit,
        )
        .await
        .unwrap();

    assert_eq!(first.len(), K, "first read returns all K rows");
    assert_eq!(second.len(), K, "second read returns all K rows");

    let first_ids: Vec<String> = first.iter().map(|a| a.id().to_owned()).collect();
    let second_ids: Vec<String> = second.iter().map(|a| a.id().to_owned()).collect();
    assert_eq!(
        first_ids, second_ids,
        "two reads of the same historical window must be identical (deterministic)"
    );

    // The window read is newest-first and stable.
    assert_eq!(first[0].id(), "h49", "newest 2022 row first");
    assert_eq!(first[K - 1].id(), "h0", "oldest 2022 row last");
}

/// Regression: an activity whose sport falls through to `SportType::Other(_)`
/// — a provider type cageux has no named variant for — must still populate the
/// indexed `sport_type` column. `SportType` is externally tagged, so `Other`
/// serializes to a JSON object `{"other": "<provider_type>"}` rather than a
/// plain string; the column write previously demanded a string and bound NULL
/// for every such row, so a `GROUP BY sport_type` reported them as `(null)`
/// even though the canonical value was intact in `data_json`. Named variants
/// stay plain `snake_case` strings; `Other` unwraps to its inner provider string.
#[tokio::test]
async fn activity_cache_other_sport_type_populates_indexed_column() {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let tenant_id = TenantId::generate();
    let repo = &repos.activity_cache;

    let start = Utc.with_ymd_and_hms(2022, 7, 1, 8, 0, 0).unwrap();
    let named = ActivityBuilder::new(
        "named".to_owned(),
        "named run".to_owned(),
        SportType::Run,
        start,
        3_600,
        "strava".to_owned(),
    )
    .build();
    // A Strava type cageux has no named variant for (e.g. an e-bike ride):
    // sciotte maps it to `SportType::Other("e_bike_ride")`.
    let other = ActivityBuilder::new(
        "other".to_owned(),
        "other ride".to_owned(),
        SportType::Other("e_bike_ride".to_owned()),
        start,
        3_600,
        "strava".to_owned(),
    )
    .build();

    repo.upsert_activities(user_id, &tenant_id, "strava", &[named, other])
        .await
        .unwrap();

    // Named variant: plain snake_case string, unchanged behaviour.
    assert_eq!(
        read_sport_column(&database, "named").await,
        Some("run".to_owned())
    );
    // `Other(_)`: the inner provider string — NOT NULL. This is the bug fixed.
    assert_eq!(
        read_sport_column(&database, "other").await,
        Some("e_bike_ride".to_owned())
    );
}

/// Read the raw indexed `sport_type` column for an activity. The public read
/// path only surfaces `data_json`, so the column is queried directly. The fix
/// lives in both the `SQLite` and `PostgreSQL` persistence layers, so this is
/// backend-aware and verifies whichever backend the suite runs against.
async fn read_sport_column(database: &Database, activity_id: &str) -> Option<String> {
    #[cfg(feature = "postgresql")]
    if let Some(pool) = database.postgres_pool() {
        return sqlx::query("SELECT sport_type FROM cached_activities WHERE activity_id = $1")
            .bind(activity_id)
            .fetch_one(pool)
            .await
            .unwrap()
            .get::<Option<String>, _>("sport_type");
    }
    let sqlite = database
        .sqlite_database()
        .expect("test database is SQLite when the postgresql feature is off");
    sqlx::query("SELECT sport_type FROM cached_activities WHERE activity_id = ?")
        .bind(activity_id)
        .fetch_one(sqlite.pool())
        .await
        .unwrap()
        .get::<Option<String>, _>("sport_type")
}

#[tokio::test]
async fn fetch_mark_advances_freshness_without_rows() {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let repos = database.repositories();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let tenant_id = TenantId::generate();
    let repo = &repos.activity_cache;

    // Never fetched: both freshness reads say so.
    assert_eq!(
        repo.latest_activity_sync(user_id, &tenant_id, "strava")
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        repo.latest_activity_sync_any(user_id, &tenant_id)
            .await
            .unwrap(),
        None
    );

    // A fetch that returned zero activities still recorded that it happened.
    let fetched_at = Utc::now();
    repo.record_activity_fetch(user_id, &tenant_id, "strava", fetched_at)
        .await
        .unwrap();

    let per_provider = repo
        .latest_activity_sync(user_id, &tenant_id, "strava")
        .await
        .unwrap()
        .expect("the mark alone is a freshness signal");
    assert_eq!(per_provider.timestamp(), fetched_at.timestamp());
    let any = repo
        .latest_activity_sync_any(user_id, &tenant_id)
        .await
        .unwrap()
        .expect("the cross-provider read sees the mark too");
    assert_eq!(any.timestamp(), fetched_at.timestamp());

    // The mark is provider-scoped: garmin has still never been fetched.
    assert_eq!(
        repo.latest_activity_sync(user_id, &tenant_id, "garmin")
            .await
            .unwrap(),
        None
    );

    // Rows arriving later win: the reads return the later of rows and mark.
    repo.upsert_activities(
        user_id,
        &tenant_id,
        "strava",
        &[activity("m1", "strava", 1)],
    )
    .await
    .unwrap();
    let after_rows = repo
        .latest_activity_sync(user_id, &tenant_id, "strava")
        .await
        .unwrap()
        .unwrap();
    assert!(
        after_rows >= per_provider,
        "row-backed synced_at postdates the earlier mark"
    );

    // An overwriting mark newer than the rows wins the other way.
    let much_later = Utc::now() + Duration::hours(2);
    repo.record_activity_fetch(user_id, &tenant_id, "strava", much_later)
        .await
        .unwrap();
    let re_marked = repo
        .latest_activity_sync_any(user_id, &tenant_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(re_marked.timestamp(), much_later.timestamp());
}
