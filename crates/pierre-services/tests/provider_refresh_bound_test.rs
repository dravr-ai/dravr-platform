// ABOUTME: The blocking refresh budget caps a wedged sync and speaks a clean sentence to the athlete
// ABOUTME: One helper backs both refresh paths, so the timeout copy is asserted once, here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `bound_blocking_sync` is what stands between a wedged provider sync and a
//! chat turn held open indefinitely. The single-provider path ran unbounded
//! until 938829b8c, and the copy that gained the cap also gained a bad line
//! wrap that put a 34-space run into the middle of the sentence the coach
//! relays — so the message is asserted here character by character, not just
//! for its numbers.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::future::{pending, ready};
use std::time::Duration;

use pierre_services::provider_refresh::{bound_blocking_sync, RefreshResult};

#[tokio::test]
async fn a_wedged_sync_is_capped_and_reads_as_one_sentence() {
    let result = bound_blocking_sync("strava", Duration::from_secs(1), pending()).await;

    assert!(
        !result.success,
        "a sync that never returned did not succeed"
    );
    assert_eq!(result.provider, "strava", "the refusal names the provider");
    assert_eq!(result.records_synced, 0);
    assert_eq!(
        result.message, "Sync did not complete within 1s; ask again shortly or retry the refresh",
        "the athlete reads this verbatim through the coach — no double spaces, no wrap damage"
    );
    assert!(
        !result.message.contains("  "),
        "a run of spaces in athlete-facing copy is the defect this pins: {:?}",
        result.message
    );
}

#[tokio::test]
async fn a_sync_that_finishes_inside_the_budget_is_returned_untouched() {
    let completed = RefreshResult {
        provider: "fitbit".to_owned(),
        success: true,
        message: "Synced 42 activities".to_owned(),
        records_synced: 42,
    };

    let result = bound_blocking_sync("fitbit", Duration::from_secs(30), ready(completed)).await;

    assert!(result.success);
    assert_eq!(result.records_synced, 42);
    assert_eq!(
        result.message, "Synced 42 activities",
        "the helper reports the sync's own verdict, it does not invent one"
    );
}

#[tokio::test]
async fn a_reported_failure_inside_the_budget_keeps_its_own_message() {
    let refused = RefreshResult {
        provider: "garmin".to_owned(),
        success: false,
        message: "Garmin rejected the credentials".to_owned(),
        records_synced: 0,
    };

    let result = bound_blocking_sync("garmin", Duration::from_secs(30), ready(refused)).await;

    assert!(!result.success);
    assert_eq!(
        result.message, "Garmin rejected the credentials",
        "a failure that arrived in time must not be relabelled as a timeout"
    );
}
