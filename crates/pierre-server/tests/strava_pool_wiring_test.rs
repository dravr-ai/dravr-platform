// ABOUTME: Verifies the Strava pool seat aggregation used by the connect recommender
// ABOUTME: A pool app's seat_cap adds to the total capacity that keeps OAuth offered before the Sciotte fallback
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use common::create_test_server_resources;
use pierre_auth::strava_pool::strava_seat_summary;

/// A pool app's `seat_cap` must add to the capacity the recommender reads.
///
/// `compute_providers_status` reads `strava_seat_summary` to decide whether to
/// keep offering Strava OAuth (`recommended_backend: oauth`) or fall back to the
/// Sciotte mirror. Adding a pool app must raise the total capacity by its
/// `seat_cap` without changing how many seats are already used — that is the
/// whole point of the pool.
#[tokio::test]
async fn pool_app_seat_cap_adds_to_total_capacity() {
    let resources = create_test_server_resources().await.unwrap();
    let repo = &resources.common.repos.oauth_tokens;

    let before = strava_seat_summary(repo.as_ref())
        .await
        .expect("seat summary");

    repo.upsert_strava_pool_app("777777", "poolsecretvaluewithlength30chr", 5, Some("app-2"))
        .await
        .expect("add pool app");

    let after = strava_seat_summary(repo.as_ref())
        .await
        .expect("seat summary");

    assert_eq!(
        after.total,
        before.total + 5,
        "an enabled pool app adds its seat_cap to the total pool capacity"
    );
    assert_eq!(
        after.used, before.used,
        "adding an empty pool app does not change occupied seats"
    );
    assert_eq!(
        after.left(),
        after.total - after.used,
        "left = total - used"
    );
    assert!(
        after.left() >= before.left() + 5,
        "the extra capacity is available as free seats"
    );

    // Disabling the app removes its capacity again.
    repo.set_strava_pool_app_enabled("777777", false)
        .await
        .unwrap();
    let disabled = strava_seat_summary(repo.as_ref()).await.unwrap();
    assert_eq!(
        disabled.total, before.total,
        "a disabled pool app contributes no capacity"
    );
}
