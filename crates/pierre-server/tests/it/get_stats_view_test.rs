// ABOUTME: get_stats as a view over the one aggregator: its totals are query_activities' totals, over every source merged
// ABOUTME: This year from the same read, every sport counted, each source on its own ladder, a walk asked where none ran

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `get_stats` (carnet#1485).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// The totals are read from the store, never from a provider: the Strava
// stand-in answers an empty history and no aggregate route at all, so a
// provider read of Strava's `/athletes/{id}/stats` would fail the call.
#![cfg(all(
    feature = "tools-data",
    feature = "provider-strava",
    feature = "provider-garmin",
    feature = "health-sync"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::activity_query_test::{
    connect_garmin, noon, owe_history_walk, query, ride, source_entry, store, store_server,
    strava_athlete, walk_asked, walked_to_its_end, workout, Athlete,
};

use std::sync::Arc;

use chrono::{Datelike, Utc};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::SportType;
use pierre_core::transport::Transport;
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::{json, Value};

use crate::activity_query_test::call_tool;

/// `get_stats` from the athlete's own web app.
async fn stats(resources: &Arc<ServerContext>, athlete: Athlete, params: Value) -> Value {
    call_tool(resources, Transport::WebApp, athlete, "get_stats", params).await
}

/// An athlete whose Strava history is walked to its end, holding a ride
/// this year, one last year and one in 2015.
async fn three_years(resources: &Arc<ServerContext>) -> Athlete {
    let athlete = strava_athlete(resources).await;
    walked_to_its_end(resources, athlete, oauth_providers::STRAVA).await;
    let year = Utc::now().year();
    let this_year = Utc::now() - chrono::Duration::hours(2);
    let rows = [
        ride(oauth_providers::STRAVA, this_year),
        ride(oauth_providers::STRAVA, noon(year - 1, 6, 1)),
        ride(oauth_providers::STRAVA, noon(2015, 6, 1)),
    ];
    store(resources, athlete, oauth_providers::STRAVA, &rows).await;
    athlete
}

#[tokio::test]
async fn the_totals_are_the_counts_query_activities_gives() {
    let resources = store_server().await;
    let athlete = three_years(&resources).await;

    let view = stats(&resources, athlete, json!({})).await;
    let all_time = query(&resources, athlete, json!({})).await;
    let jan_first = format!("{}-01-01", Utc::now().year());
    let this_year = query(&resources, athlete, json!({ "from": jan_first })).await;

    assert_eq!(
        view["stats"]["total_activities"], all_time["totals"]["count"],
        "bug: get_stats counts other than query_activities: {view} vs {all_time}"
    );
    assert_eq!(view["stats"]["total_activities"], 3, "{view}");
    assert_eq!(
        view["stats"]["total_distance"], 90_000.0,
        "bug: the all-time distance is not every source's metres: {view}"
    );
    assert_eq!(
        view["stats"]["year_to_date"]["total_activities"], this_year["totals"]["count"],
        "bug: the year to date is not query_activities from Jan 1: {view} vs {this_year}"
    );
    assert_ne!(
        view["stats"]["year_to_date"]["total_activities"], view["stats"]["total_activities"],
        "bug: the year to date repeats the all-time total: {view}"
    );
    assert_eq!(
        view["stats"]["coverage"]["window"]["scope"]["kind"], "asked",
        "bug: the view's window is no asked window: {view}"
    );
    assert_eq!(
        view["stats"]["coverage"]["complete"], true,
        "bug: a history walked to its feed end reads short: {view}"
    );
}

#[tokio::test]
async fn every_sport_counts_not_strava_s_ride_and_run_buckets() {
    let resources = store_server().await;
    let athlete = strava_athlete(&resources).await;
    walked_to_its_end(&resources, athlete, oauth_providers::STRAVA).await;
    let rows = [
        workout(
            oauth_providers::STRAVA,
            "mtb",
            SportType::MountainBike,
            noon(2024, 5, 1),
        ),
        workout(
            oauth_providers::STRAVA,
            "trail",
            SportType::TrailRunning,
            noon(2024, 5, 2),
        ),
        workout(
            oauth_providers::STRAVA,
            "zwift",
            SportType::VirtualRide,
            noon(2024, 5, 3),
        ),
    ];
    store(&resources, athlete, oauth_providers::STRAVA, &rows).await;

    let view = stats(&resources, athlete, json!({})).await;

    assert_eq!(
        view["stats"]["total_activities"], 3,
        "bug: a mountain-bike ride, a trail run or a virtual ride is left out: {view}"
    );
}

#[tokio::test]
async fn two_sources_each_walked_to_their_own_feed_end_are_complete() {
    let resources = store_server().await;
    let athlete = strava_athlete(&resources).await;
    walked_to_its_end(&resources, athlete, oauth_providers::STRAVA).await;
    store(
        &resources,
        athlete,
        oauth_providers::STRAVA,
        &[ride(oauth_providers::STRAVA, noon(2015, 4, 1))],
    )
    .await;
    connect_garmin(&resources, athlete).await;
    walked_to_its_end(&resources, athlete, oauth_providers::GARMIN).await;
    store(
        &resources,
        athlete,
        oauth_providers::GARMIN,
        &[workout(
            oauth_providers::GARMIN,
            "g-22",
            SportType::Run,
            noon(2022, 4, 1),
        )],
    )
    .await;

    let view = stats(&resources, athlete, json!({})).await;

    assert_eq!(
        view["stats"]["coverage"]["complete"], true,
        "bug: a source connected later reads as a gap against an older one: {view}"
    );
}

#[tokio::test]
async fn a_source_owed_its_walk_with_nothing_stored_still_makes_the_totals_short() {
    let resources = store_server().await;
    let athlete = strava_athlete(&resources).await;
    owe_history_walk(&resources, athlete, oauth_providers::STRAVA, (1, 30)).await;

    let view = stats(&resources, athlete, json!({})).await;

    assert_eq!(
        view["stats"]["coverage"]["incomplete_reason"], "history_walk_running",
        "bug: a source with no rows yet and a walk owed dropped out of the totals: {view}"
    );
    let strava = source_entry(&view, "strava");
    assert_eq!(
        strava["copies"], 0,
        "bug: the source is listed with the copies it holds: {strava}"
    );
    assert_eq!(
        strava["coverage"]["pending"]["months_done"], 1,
        "bug: the source carries its own walk's progress: {strava}"
    );
}

#[tokio::test]
async fn a_never_walked_source_is_asked_its_walk_by_the_view() {
    let resources = store_server().await;
    let athlete = strava_athlete(&resources).await;
    store(
        &resources,
        athlete,
        oauth_providers::STRAVA,
        &[ride(oauth_providers::STRAVA, noon(2025, 5, 1))],
    )
    .await;

    let view = stats(&resources, athlete, json!({})).await;

    assert_eq!(
        view["stats"]["coverage"]["pending"]["kind"], "history_not_walked",
        "bug: a never-walked Strava reads as its few stored rows: {view}"
    );
    assert!(
        walk_asked(&resources, athlete, oauth_providers::STRAVA).await,
        "bug: the view asked no Strava walk"
    );
}

#[tokio::test]
async fn provider_counts_one_source_alone() {
    let resources = store_server().await;
    let athlete = strava_athlete(&resources).await;
    walked_to_its_end(&resources, athlete, oauth_providers::STRAVA).await;
    connect_garmin(&resources, athlete).await;
    store(
        &resources,
        athlete,
        oauth_providers::STRAVA,
        &[ride(oauth_providers::STRAVA, noon(2024, 5, 1))],
    )
    .await;
    store(
        &resources,
        athlete,
        oauth_providers::GARMIN,
        &[workout(
            oauth_providers::GARMIN,
            "g-run",
            SportType::Run,
            noon(2024, 7, 1),
        )],
    )
    .await;

    let both = stats(&resources, athlete, json!({})).await;
    let strava = stats(&resources, athlete, json!({ "provider": "strava" })).await;

    assert_eq!(both["stats"]["total_activities"], 2, "{both}");
    assert_eq!(
        strava["stats"]["total_activities"], 1,
        "bug: provider is no source filter: {strava}"
    );
}
