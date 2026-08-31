// ABOUTME: A live connection served from cache while its provider went quiet must be reportable
// ABOUTME: Pins the divergence rule and the cross-tenant snapshot read behind /admin/diagnostics
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! jf@dravr.ai's sciotte capture stopped after 2026-08-28 02:59Z and was still
//! stopped two days later. Three real activities never landed. Nothing noticed,
//! because `activity_fetch_freshness` — which held the honest time of the last
//! successful provider fetch the whole time — was read by nothing except a
//! per-user freshness report nobody was looking at (carnet#149).
//!
//! What makes the failure detectable is a DIVERGENCE, not an age.
//! `provider_connections.last_used_at` is touched at the serve chokepoint on
//! every serve, including one the durable cache answered;
//! `activity_fetch_freshness.fetched_at` advances only when a fetch genuinely
//! reached the provider. Recently served AND long unfetched is an athlete being
//! answered from a frozen cache.
//!
//! These tests pin both halves: the rule that reads the divergence, and the
//! cross-tenant snapshot that feeds it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use chrono::{DateTime, Duration, Utc};
use pierre_core::models::ConnectionType;
use pierre_database::repositories::CaptureFreshness;
use pierre_mcp_server::routes::admin::diagnostics::partition_stale_captures;

use crate::common::{create_test_server_resources, create_test_user};

/// Build one snapshot row with ages expressed relative to `now`.
fn row(
    provider: &str,
    now: DateTime<Utc>,
    used_hours_ago: Option<i64>,
    fetched_hours_ago: Option<i64>,
) -> CaptureFreshness {
    CaptureFreshness {
        tenant_id: "11111111-1111-1111-1111-111111111111".to_owned(),
        user_id: "22222222-2222-2222-2222-222222222222".to_owned(),
        provider: provider.to_owned(),
        last_used_at: used_hours_ago.map(|h| now - Duration::hours(h)),
        last_fetch_at: fetched_hours_ago.map(|h| now - Duration::hours(h)),
    }
}

/// The incident shape itself: served an hour ago, last real fetch three days
/// back. This is the row that must come out, and it must carry the honest age.
#[test]
fn a_connection_served_from_a_frozen_cache_is_reported_with_its_true_age() {
    let now = Utc::now();
    let snapshot = vec![row("sciotte", now, Some(1), Some(72))];

    let (judged, stale) =
        partition_stale_captures(&snapshot, now, Duration::hours(24), Duration::hours(48));

    assert_eq!(judged, 1, "a connection served an hour ago must be judged");
    assert_eq!(
        stale.len(),
        1,
        "72h since a fetch is past the 24h threshold"
    );
    assert_eq!(stale[0].provider, "sciotte");
    let hours = stale[0]
        .hours_since_fetch
        .expect("a connection that has fetched reports its age");
    assert!(
        (hours - 72.0).abs() < 0.1,
        "expected ~72h since the last fetch, got {hours}"
    );
}

/// A healthy connection: served recently, fetched recently. Reporting this
/// would train an operator to ignore the alert.
#[test]
fn a_connection_still_reaching_its_provider_is_not_reported() {
    let now = Utc::now();
    let snapshot = vec![row("strava", now, Some(1), Some(2))];

    let (judged, stale) =
        partition_stale_captures(&snapshot, now, Duration::hours(24), Duration::hours(48));

    assert_eq!(judged, 1);
    assert!(
        stale.is_empty(),
        "a fetch 2h old is inside the 24h threshold, got {stale:?}"
    );
}

/// The false positive this design exists to avoid. An athlete who has not opened
/// the app in a week has nothing that SHOULD have fetched, so their ancient
/// `last_fetch_at` is correct rather than alarming — and they must not be judged
/// at all, or every dormant account becomes a page.
#[test]
fn an_athlete_who_has_not_asked_anything_is_never_judged() {
    let now = Utc::now();
    let snapshot = vec![row("whoop", now, Some(24 * 7), Some(24 * 7))];

    let (judged, stale) =
        partition_stale_captures(&snapshot, now, Duration::hours(24), Duration::hours(48));

    assert_eq!(
        judged, 0,
        "a connection last served a week ago is outside the 48h activity window"
    );
    assert!(stale.is_empty());
}

/// A connection that has served an athlete without ever recording one successful
/// fetch is the most alarming state available, and it is distinct from a large
/// age — flattening the two would hide which one it is.
#[test]
fn a_connection_that_never_fetched_is_stale_and_says_so_distinctly() {
    let now = Utc::now();
    let snapshot = vec![row("garmin", now, Some(2), None)];

    let (judged, stale) =
        partition_stale_captures(&snapshot, now, Duration::hours(24), Duration::hours(48));

    assert_eq!(judged, 1);
    assert_eq!(stale.len(), 1);
    assert!(
        stale[0].hours_since_fetch.is_none(),
        "never-fetched must stay None, not be flattened into a number"
    );
    assert!(stale[0].last_fetch_at.is_none());
}

/// A connection that has never served anyone is not evidence of anything.
#[test]
fn a_connection_that_never_served_is_never_judged() {
    let now = Utc::now();
    let snapshot = vec![row("coros", now, None, None)];

    let (judged, stale) =
        partition_stale_captures(&snapshot, now, Duration::hours(24), Duration::hours(48));

    assert_eq!(judged, 0);
    assert!(stale.is_empty());
}

/// The thresholds are the caller's, and moving them must actually move the
/// verdict — that is what lets an operator re-ask with a tighter question
/// without a deploy.
#[test]
fn the_threshold_decides_the_verdict() {
    let now = Utc::now();
    let snapshot = vec![row("sciotte", now, Some(1), Some(6))];

    let (_, lenient) =
        partition_stale_captures(&snapshot, now, Duration::hours(24), Duration::hours(48));
    assert!(lenient.is_empty(), "6h is inside a 24h threshold");

    let (_, strict) =
        partition_stale_captures(&snapshot, now, Duration::hours(4), Duration::hours(48));
    assert_eq!(strict.len(), 1, "6h is outside a 4h threshold");
}

/// A mixed population must be counted honestly: three judged, one of them stale.
#[test]
fn a_mixed_population_reports_honest_counts() {
    let now = Utc::now();
    let snapshot = vec![
        row("sciotte", now, Some(1), Some(72)), // judged, stale
        row("strava", now, Some(3), Some(3)),   // judged, healthy
        row("whoop", now, Some(6), Some(10)),   // judged, healthy
        row("garmin", now, Some(24 * 9), None), // dormant, not judged
        row("coros", now, None, None),          // never served, not judged
    ];

    let (judged, stale) =
        partition_stale_captures(&snapshot, now, Duration::hours(24), Duration::hours(48));

    assert_eq!(judged, 3, "three connections served inside the window");
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].provider, "sciotte");
}

/// The snapshot read itself: it must pair a real connection with the fetch mark
/// recorded against it, across the two tables that store their identifiers
/// differently.
#[tokio::test]
async fn the_snapshot_pairs_a_connection_with_its_fetch_mark() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let tenants = resources
        .coach
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .expect("list tenants");
    let tenant = tenants.first().expect("user has a tenant").id;

    let connections = &resources.common.repos.provider_connections;
    let cache = &resources.common.repos.activity_cache;

    // A connection that has served AND fetched.
    connections
        .register_connection(user_id, tenant, "sciotte", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    connections
        .touch_last_used(user_id, tenant, "sciotte")
        .await
        .unwrap();
    let fetched_at = Utc::now() - Duration::hours(50);
    cache
        .record_activity_fetch(user_id, &tenant, "sciotte", fetched_at)
        .await
        .unwrap();

    // A connection that has served but never fetched.
    connections
        .register_connection(user_id, tenant, "whoop", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    connections
        .touch_last_used(user_id, tenant, "whoop")
        .await
        .unwrap();

    let snapshot = cache.capture_freshness_snapshot(100).await.unwrap();

    let sciotte = snapshot
        .iter()
        .find(|c| c.provider == "sciotte")
        .expect("the sciotte connection is in the snapshot");
    assert_eq!(sciotte.user_id, user_id.to_string());
    assert_eq!(sciotte.tenant_id, tenant.to_string());
    assert!(
        sciotte.last_used_at.is_some(),
        "a touched connection carries its serve time"
    );
    let mark = sciotte
        .last_fetch_at
        .expect("the recorded fetch mark is joined onto the connection");
    assert!(
        (mark - fetched_at).num_seconds().abs() <= 1,
        "expected the mark at {fetched_at}, got {mark}"
    );

    let whoop = snapshot
        .iter()
        .find(|c| c.provider == "whoop")
        .expect("the whoop connection is in the snapshot");
    assert!(
        whoop.last_fetch_at.is_none(),
        "a connection with no fetch mark must read as never-fetched, not be dropped"
    );
    assert!(
        whoop.last_used_at.is_some(),
        "NULL handling must not swallow a real serve time"
    );

    // And the divergence rule turns that snapshot into exactly one finding: the
    // sciotte row is 50h past its last fetch, the whoop row never fetched at all.
    let (judged, stale) = partition_stale_captures(
        &snapshot,
        Utc::now(),
        Duration::hours(24),
        Duration::hours(48),
    );
    assert_eq!(judged, 2, "both connections were served just now");
    assert_eq!(stale.len(), 2, "one frozen, one never fetched");
}

/// A connection needing re-auth has a KNOWN reason to have stopped fetching and
/// already surfaces through the reconnect path. Leaving it in would bury the
/// silent failures this endpoint exists to find under a pile of loud ones.
#[tokio::test]
async fn a_connection_needing_reauth_is_excluded_from_the_snapshot() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let tenants = resources
        .coach
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .expect("list tenants");
    let tenant = tenants.first().expect("user has a tenant").id;

    let connections = &resources.common.repos.provider_connections;
    connections
        .register_connection(user_id, tenant, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    connections
        .touch_last_used(user_id, tenant, "strava")
        .await
        .unwrap();

    let before = resources
        .common
        .repos
        .activity_cache
        .capture_freshness_snapshot(100)
        .await
        .unwrap();
    assert_eq!(
        before.iter().filter(|c| c.provider == "strava").count(),
        1,
        "an active connection is in the snapshot to begin with"
    );

    connections
        .mark_needs_reauth(user_id, tenant, "strava", Some("invalid_grant"))
        .await
        .unwrap();

    let after = resources
        .common
        .repos
        .activity_cache
        .capture_freshness_snapshot(100)
        .await
        .unwrap();
    assert_eq!(
        after.iter().filter(|c| c.provider == "strava").count(),
        0,
        "a connection needing re-auth must drop out of the snapshot"
    );
}
