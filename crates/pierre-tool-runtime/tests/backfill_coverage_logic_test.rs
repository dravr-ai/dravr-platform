// ABOUTME: Unit tests for the historical-gate pure decisions — depth-coverage, feed-end, response-cache eligibility, list ordering
// ABOUTME: The decisions behind the self-healing gate (serve vs re-backfill), what bypasses the response cache, and sort_by ordering
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]
#![allow(clippy::unwrap_used)]

use chrono::{TimeZone, Utc};
use pierre_core::models::{Activity, ActivityBuilder, SportType};
use pierre_database::repositories::BackfillCoverage;
use pierre_tool_runtime::activity_backfill::backfill_covered_floor_ts;
use pierre_tool_runtime::implementations::data::{
    historical_depth_covered, response_cache_eligible, sort_activities,
};

// Jan 1 2022 and a few reference points (unix seconds).
const JAN_2022: i64 = 1_640_995_200;
const JUL_2022: i64 = 1_657_000_000; // mid-2022, newer than JAN_2022
const DEC_2021: i64 = 1_640_000_000; // older than JAN_2022
const JAN_2024: i64 = 1_704_067_200; // Jan 1 2024 00:00:00

#[test]
fn no_coverage_record_is_not_covered() {
    // Never backfilled — the cached rows (if any) are an unverified slice.
    assert!(!historical_depth_covered(None, JAN_2022));
}

#[test]
fn backfill_reached_the_requested_floor_is_covered() {
    // Reached exactly the requested floor: oldest_reached <= after.
    let c = BackfillCoverage {
        oldest_reached_ts: JAN_2022,
        hit_feed_end: false,
    };
    assert!(historical_depth_covered(Some(c), JAN_2022));
}

#[test]
fn backfill_reached_deeper_than_requested_is_covered() {
    let c = BackfillCoverage {
        oldest_reached_ts: DEC_2021,
        hit_feed_end: false,
    };
    assert!(historical_depth_covered(Some(c), JAN_2022));
}

#[test]
fn shallow_backfill_short_of_floor_is_not_covered() {
    // Oldest reached is July 2022 but the request asks back to January and the
    // feed did NOT end — exactly the "2022 starts in July" partial. Re-backfill.
    let c = BackfillCoverage {
        oldest_reached_ts: JUL_2022,
        hit_feed_end: false,
    };
    assert!(!historical_depth_covered(Some(c), JAN_2022));
}

#[test]
fn feed_end_short_of_floor_is_covered() {
    // Oldest reached is July 2022, short of the January request, BUT the feed
    // ended there — the athlete genuinely has no older data, so it is covered
    // and must NOT loop re-scraping.
    let c = BackfillCoverage {
        oldest_reached_ts: JUL_2022,
        hit_feed_end: true,
    };
    assert!(historical_depth_covered(Some(c), JAN_2022));
}

#[test]
fn covered_floor_is_the_requested_after_when_not_count_capped() {
    // A year query asks back to Jan 1 00:00 but the oldest activity sits in
    // mid-year; the scrape was not count-capped, so it returned the WHOLE
    // window. The covered floor is the requested `after` (Jan 2022), NOT the
    // oldest activity (Jul 2022) — otherwise the same year re-scrapes forever
    // because oldest > after.
    assert_eq!(
        backfill_covered_floor_ts(JAN_2022, JUL_2022, 46, 2_000),
        JAN_2022
    );
}

#[test]
fn covered_floor_is_the_oldest_activity_when_count_capped() {
    // Filled the fetch limit: the scrape stopped early, so it can only claim
    // down to the oldest activity it actually fetched, not the requested floor.
    assert_eq!(
        backfill_covered_floor_ts(JAN_2022, JUL_2022, 2_000, 2_000),
        JUL_2022
    );
}

#[test]
fn covered_floor_clamps_to_oldest_when_scrape_overshoots_the_floor() {
    // Defensive: if a scrape returns an activity older than the requested
    // `after`, the deeper of the two is covered.
    assert_eq!(
        backfill_covered_floor_ts(JAN_2022, DEC_2021, 200, 2_000),
        DEC_2021
    );
}

#[test]
fn shallow_year_coverage_does_not_cover_a_deeper_year() {
    // Regression for the "No push / 2022 served empty" outage: a 2024-bounded
    // backfill records covered floor = Jan 2024. A later 2022 ask must NOT read
    // as covered (oldest_reached_ts > after, hit_feed_end false) — it has to
    // re-backfill and push, not serve an empty slice from the 2024-only cache.
    let c = BackfillCoverage {
        oldest_reached_ts: JAN_2024,
        hit_feed_end: false,
    };
    assert!(!historical_depth_covered(Some(c), JAN_2022));
}

#[test]
fn recent_summary_query_uses_the_response_cache() {
    // Not auto-promoted, not historical, default sort: the ordinary recent
    // path — the response cache is its only read-cache between a turn and a
    // live fetch.
    assert!(response_cache_eligible(false, false, false));
}

#[test]
fn historical_query_bypasses_the_response_cache() {
    // Regression for 9bff5a72a: a deep historical `after` must NOT consult the
    // response cache — a TTL'd hit short-circuited the coverage-aware gate and
    // kept serving the stale slice after the coverage purge ("2022 stuck at
    // Jul–Dec"). Excluding it from the WRITE too stops dead never-read entries.
    assert!(!response_cache_eligible(false, true, false));
}

#[test]
fn detail_promoted_query_bypasses_the_response_cache() {
    // The cache key omits `mode`, so a detail-promoted payload must neither be
    // read from nor written under a key a summary request would use.
    assert!(!response_cache_eligible(true, false, false));
}

#[test]
fn detail_promoted_historical_query_bypasses_the_response_cache() {
    // Both exclusions at once still bypasses the cache.
    assert!(!response_cache_eligible(true, true, false));
}

#[test]
fn custom_sort_query_bypasses_the_response_cache() {
    // The cache key omits sort_by and the cached-serve path re-orders by date,
    // so a "longest to shortest" (non-default sort) ask must bypass the cache —
    // otherwise it could serve a date-ordered cached slice for a distance sort.
    assert!(!response_cache_eligible(false, false, true));
}

/// Build a Run activity with an explicit id, start date, distance (km) and
/// duration (s) so an ordering test can place it deterministically.
fn act(id: &str, start_ts: i64, km: f64, duration_s: u64) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        id.to_owned(),
        SportType::Run,
        Utc.timestamp_opt(start_ts, 0).single().unwrap(),
        duration_s,
        "strava".to_owned(),
    )
    .distance_meters(km * 1000.0)
    .build()
}

fn ids(activities: &[Activity]) -> Vec<String> {
    activities.iter().map(|a| a.id().to_owned()).collect()
}

#[test]
fn sort_activities_honors_distance_date_and_duration() {
    // a: newest, mid distance, mid duration | b: oldest, longest, longest time
    // | c: middle date, shortest, shortest time.
    let base = vec![
        act("a", JAN_2024, 10.0, 3_600),
        act("b", DEC_2021, 42.0, 14_400),
        act("c", JUL_2022, 5.0, 1_800),
    ];

    let sorted = |key: &str| {
        let mut v = base.clone();
        sort_activities(&mut v, key);
        ids(&v)
    };

    // Distance: longest→shortest and the reverse.
    assert_eq!(sorted("distance_desc"), ["b", "a", "c"]);
    assert_eq!(sorted("distance_asc"), ["c", "a", "b"]);
    // Date: newest→oldest (default) and oldest→newest.
    assert_eq!(sorted("date_desc"), ["a", "c", "b"]);
    assert_eq!(sorted("date_asc"), ["b", "c", "a"]);
    // Duration: longest→shortest and the reverse.
    assert_eq!(sorted("duration_desc"), ["b", "a", "c"]);
    assert_eq!(sorted("duration_asc"), ["c", "a", "b"]);
    // Unknown key falls back to newest-first (date_desc).
    assert_eq!(sorted("bogus"), ["a", "c", "b"]);
}
