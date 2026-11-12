// ABOUTME: carnet#1482 — the best-effort seed reads the athlete's history from the store a history walk filled, never a listing
// ABOUTME: Before the walk ends it waits; after, each run's streams still come from the Strava grant, a session's twin measured once

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The personal-best seed on the activity store (carnet#1482).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// A Strava athlete holding the usr/pwd session beside the grant is walked
// through the session, so their history lands in the store under `sciotte`
// and the walk's row is the session's. The seed reads that store: it never
// lists the athlete's activities at Strava, and it measures each run on the
// grant's streams, the session declaring no distance channel. A ride stored
// under both keys is one workout, measured once. Until a walk under either
// key has ended, the seed waits: no request, no cursor, not complete.
//
// The Strava stand-in is `personal_best_seed_test`'s, which counts every
// listing, streams request and profile read it answers.
#![cfg(all(
    feature = "health-sync",
    feature = "provider-strava",
    feature = "client-notifications"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::helpers::history_walk_ledger::{end_history_walk, owe_history_walk, walked_history};
use crate::personal_best_seed_test::{
    connected_athlete, context_pointed_at, mock_strava, open_window, recorded, run, sync_scan,
    synced, walk_complete, Athlete, MockActivity,
};

use std::sync::Arc;

use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::Activity;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::personal_best_seed::{
    exact_provider, run_personal_best_seed_pass, SeedPassReport,
};
use pierre_services::personal_bests::{seed_stream_source, seed_stream_sources, SeedProgress};

/// Land `activities` in the athlete's store under the usr/pwd session, as
/// a walk through it writes them: Strava's own ids, Strava as the source.
async fn store_as_session(
    resources: &ServerContext,
    athlete: &Athlete,
    activities: &[MockActivity],
) {
    let rows: Vec<Activity> = activities
        .iter()
        .map(|mocked| {
            recorded(
                *mocked,
                oauth_providers::SCIOTTE,
                Some(oauth_providers::STRAVA),
            )
        })
        .collect();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(
            athlete.user_id,
            &athlete.tenant_id,
            oauth_providers::SCIOTTE,
            &rows,
        )
        .await
        .unwrap();
}

/// Land `activities` in the athlete's store under the grant.
async fn store_as_grant(resources: &ServerContext, athlete: &Athlete, activities: &[MockActivity]) {
    let rows: Vec<Activity> = activities.iter().copied().map(synced).collect();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(
            athlete.user_id,
            &athlete.tenant_id,
            oauth_providers::STRAVA,
            &rows,
        )
        .await
        .unwrap();
}

/// Advance the athlete's seed once, directly, through their Strava grant.
async fn advance(resources: &Arc<ServerContext>, athlete: &Athlete) -> SeedProgress {
    let grant = exact_provider(
        resources,
        oauth_providers::STRAVA,
        athlete.user_id,
        &athlete.tenant_id.to_string(),
    )
    .await
    .expect("the athlete's Strava grant");
    resources
        .personal_bests()
        .advance_seed(grant.as_ref(), athlete.user_id, athlete.tenant_id)
        .await
        .unwrap()
}

/// Whether the seed of the athlete's Strava history has a saved row at all.
async fn seed_started(resources: &ServerContext, athlete: &Athlete) -> bool {
    resources
        .common
        .repos
        .personal_bests
        .personal_best_seed(athlete.user_id, athlete.tenant_id, oauth_providers::STRAVA)
        .await
        .unwrap()
        .is_some()
}

/// A Strava history is measured through the grant: the session declares
/// only its route, so it never serves a best effort's distance channel.
#[tokio::test]
async fn a_strava_history_is_measured_on_the_grants_streams() {
    let (api_base, _mock) = mock_strava().await;
    let resources = context_pointed_at(&api_base, 100).await;
    let registry = resources.fitness.provider_registry.as_ref();
    assert_eq!(
        seed_stream_source(registry, oauth_providers::STRAVA),
        Some(oauth_providers::STRAVA)
    );
    assert_eq!(
        seed_stream_source(registry, oauth_providers::SCIOTTE),
        Some(oauth_providers::STRAVA),
        "the session's half of the pair names the grant too"
    );
    let sources = seed_stream_sources(registry);
    assert!(sources.contains(&oauth_providers::STRAVA));
    assert!(
        !sources.contains(&oauth_providers::SCIOTTE),
        "a session is never a stream source"
    );
}

/// After a walk through the session ended, the seed reads the history the
/// session stored, never listing it at Strava, and asks the grant for each
/// run's streams once: a run stored under both keys is one workout. A sync
/// scan of that run later asks for nothing, the scan being the workout's.
#[tokio::test]
async fn after_the_walk_the_seed_reads_the_store_and_streams_through_the_grant() {
    let (api_base, mock) = mock_strava().await;
    let resources = context_pointed_at(&api_base, 100).await;
    let session_runs = [
        run(9_001, 300, 310),
        run(9_002, 200, 280),
        run(9_003, 100, 300),
    ];
    let twin = run(9_004, 20, 290);
    let mut history = session_runs.to_vec();
    history.push(twin);
    let athlete = connected_athlete(&resources, &mock, "store_seed@example.com", 41, history).await;
    let mut session_history = session_runs.to_vec();
    session_history.push(twin);
    store_as_session(&resources, &athlete, &session_history).await;
    store_as_grant(&resources, &athlete, &[twin]).await;
    walked_history(
        &resources.common.repos,
        athlete.user_id,
        athlete.tenant_id,
        oauth_providers::SCIOTTE,
    )
    .await;

    open_window(&resources).await;
    let report = run_personal_best_seed_pass(&resources).await.unwrap();
    assert_eq!(
        report,
        SeedPassReport {
            completed: 1,
            ..SeedPassReport::default()
        }
    );
    assert_eq!(
        mock.total_list_hits(),
        0,
        "the store is read, Strava never listed"
    );
    for measured in session_runs.iter().chain([&twin]) {
        assert_eq!(
            mock.streams_hits(measured.id),
            1,
            "run {} is measured once, on the grant's streams",
            measured.id
        );
    }
    assert_eq!(mock.total_streams_hits(), 4, "the twin is one workout");
    assert!(walk_complete(&resources, &athlete).await);
    let best = resources
        .common
        .repos
        .personal_bests
        .personal_bests(athlete.user_id, athlete.tenant_id)
        .await
        .unwrap()
        .into_iter()
        .find(|best| best.distance == "5k")
        .map(|best| best.elapsed_seconds);
    assert_eq!(best, Some(1400.0), "run 9002's 5 km at 4:40/km");

    sync_scan(&resources, &athlete, &[synced(twin)]).await;
    assert_eq!(
        mock.streams_hits(twin.id),
        1,
        "the sync finds the workout the seed scanned from the session's copy"
    );
}

/// While the walk is owed, and while no walk was ever asked, the seed waits:
/// no athlete is a candidate, a direct advance makes no request and saves no
/// cursor, and the seed is not complete. Once the walk ends the next pass
/// seeds the history.
#[tokio::test]
async fn the_seed_waits_until_the_walk_has_ended() {
    let (api_base, mock) = mock_strava().await;
    let resources = context_pointed_at(&api_base, 100).await;
    let runs = [run(9_101, 90, 300), run(9_102, 30, 290)];
    let athlete = connected_athlete(
        &resources,
        &mock,
        "store_wait@example.com",
        42,
        runs.to_vec(),
    )
    .await;
    store_as_grant(&resources, &athlete, &runs).await;

    // No walk asked yet.
    open_window(&resources).await;
    assert_eq!(
        run_personal_best_seed_pass(&resources).await.unwrap(),
        SeedPassReport::default(),
        "an athlete no walk filled the store for is no candidate"
    );
    assert_eq!(
        advance(&resources, &athlete).await,
        SeedProgress::AwaitingHistory
    );
    assert_eq!(mock.total_requests(), 0, "a waiting seed makes no request");
    assert!(
        !seed_started(&resources, &athlete).await,
        "no cursor is saved"
    );

    // The walk is running.
    let lease = owe_history_walk(
        &resources.common.repos,
        athlete.user_id,
        athlete.tenant_id,
        oauth_providers::STRAVA,
    )
    .await;
    assert_eq!(
        run_personal_best_seed_pass(&resources).await.unwrap(),
        SeedPassReport::default(),
        "an athlete whose walk runs is no candidate"
    );
    assert_eq!(
        advance(&resources, &athlete).await,
        SeedProgress::AwaitingHistory
    );
    assert_eq!(mock.total_requests(), 0, "a waiting seed makes no request");
    assert!(
        !seed_started(&resources, &athlete).await,
        "no cursor is saved"
    );
    assert!(!walk_complete(&resources, &athlete).await);

    // The walk reached the end of the history.
    end_history_walk(
        &resources.common.repos,
        athlete.user_id,
        athlete.tenant_id,
        oauth_providers::STRAVA,
        &lease,
    )
    .await;
    let report = run_personal_best_seed_pass(&resources).await.unwrap();
    assert_eq!(report.completed, 1);
    for measured in runs {
        assert_eq!(mock.streams_hits(measured.id), 1, "run {}", measured.id);
    }
    assert_eq!(mock.total_list_hits(), 0, "never a listing");
    assert!(walk_complete(&resources, &athlete).await);
}
