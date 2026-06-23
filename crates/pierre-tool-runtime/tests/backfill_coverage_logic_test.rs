// ABOUTME: Unit tests for the historical-gate pure decisions — depth-coverage, feed-end, response-cache eligibility
// ABOUTME: The decisions behind the self-healing gate (serve vs re-backfill) and what bypasses the response cache
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs)]

use pierre_database::repositories::BackfillCoverage;
use pierre_tool_runtime::activity_backfill::backfill_hit_feed_end;
use pierre_tool_runtime::implementations::data::{
    historical_depth_covered, response_cache_eligible,
};

// Jan 1 2022 and a few reference points (unix seconds).
const JAN_2022: i64 = 1_640_995_200;
const JUL_2022: i64 = 1_657_000_000; // mid-2022, newer than JAN_2022
const DEC_2021: i64 = 1_640_000_000; // older than JAN_2022

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
fn feed_end_when_paged_out_short_without_filling_limit() {
    // Paged past the recent window, oldest still newer than `after`, and the
    // fetch did not fill the limit => the feed ran out (no older data).
    assert!(backfill_hit_feed_end(JUL_2022, JAN_2022, 46, 2_000));
}

#[test]
fn not_feed_end_when_reached_the_floor() {
    // Oldest reached <= after: it stopped because it reached the requested
    // floor, not because the feed ended.
    assert!(!backfill_hit_feed_end(DEC_2021, JAN_2022, 200, 2_000));
}

#[test]
fn not_feed_end_when_count_capped_at_the_limit() {
    // Filled the fetch limit: the scrape was count-capped, not exhausted — a
    // deeper window might still exist, so do not claim feed-end.
    assert!(!backfill_hit_feed_end(JUL_2022, JAN_2022, 2_000, 2_000));
}

#[test]
fn recent_summary_query_uses_the_response_cache() {
    // Not auto-promoted and not historical: the ordinary recent path — the
    // response cache is its only read-cache between a turn and a live fetch.
    assert!(response_cache_eligible(false, false));
}

#[test]
fn historical_query_bypasses_the_response_cache() {
    // Regression for 9bff5a72a: a deep historical `after` must NOT consult the
    // response cache — a TTL'd hit short-circuited the coverage-aware gate and
    // kept serving the stale slice after the coverage purge ("2022 stuck at
    // Jul–Dec"). Excluding it from the WRITE too stops dead never-read entries.
    assert!(!response_cache_eligible(false, true));
}

#[test]
fn detail_promoted_query_bypasses_the_response_cache() {
    // The cache key omits `mode`, so a detail-promoted payload must neither be
    // read from nor written under a key a summary request would use.
    assert!(!response_cache_eligible(true, false));
}

#[test]
fn detail_promoted_historical_query_bypasses_the_response_cache() {
    // Both exclusions at once still bypasses the cache.
    assert!(!response_cache_eligible(true, true));
}
