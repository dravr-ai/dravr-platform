// ABOUTME: GET /api/me/activities/recent — newest cached activities across providers, clamped, tenant-scoped, cache-first
// ABOUTME: A stale cache answers at once, says so, and refreshes once in the background through the recent-fetch path

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Athlete Home recent-activities suite.
//!
//! Every test seeds real rows through the activity cache's own writer and
//! reads the route's JSON back, so a handler that answered an empty list, a
//! fixed row or the wrong tenant's rows fails on content.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
#![cfg(feature = "provider-strava")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::collections::HashMap;
use std::env;
use std::slice;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

use axum::body::{to_bytes, Body};
use axum::extract::Query;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Duration, TimeZone, Utc};
use futures_util::future::join_all;
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{
    Activity, ActivityBuilder, ConnectionType, SportType, Tenant, TenantId, User, UserOAuthToken,
};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::StoredRouteTrack;
use pierre_fitness_compute::{
    encode_polyline, trimmed_overview_polyline, RouteTrack, RouteTrackError,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use pierre_mcp_server::services::activity_route::RouteGeometrySource;
use serde_json::{json, Value};
use serial_test::serial;
use tokio::net::TcpListener;
use tokio::time::sleep;
use tower::ServiceExt;
use uuid::Uuid;

/// Every key a Home activity row carries, null or not.
const HOME_ACTIVITY_KEYS: [&str; 11] = [
    "id",
    "provider",
    "name",
    "sport_type",
    "start_date",
    "duration_seconds",
    "distance_meters",
    "elevation_gain_meters",
    "has_gps",
    "summary_polyline",
    "attribution",
];

/// One athlete: the account, the tenant they act in and a bearer for it.
struct Athlete {
    user: User,
    user_id: Uuid,
    tenant: TenantId,
    token: String,
}

async fn seed_athlete(resources: &Arc<ServerContext>, label: &str) -> Athlete {
    let email = format!("{label}-{}@example.com", Uuid::new_v4());
    let (user_id, user, tenant) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .unwrap();
    let token = common::generate_test_token(resources, &user).await;
    Athlete {
        user,
        user_id,
        tenant,
        token,
    }
}

/// A second tenant the athlete owns, and a bearer acting in it.
async fn second_tenant(resources: &Arc<ServerContext>, athlete: &Athlete) -> (TenantId, String) {
    let tenant = TenantId::generate();
    resources
        .common
        .repos
        .tenants
        .create(&Tenant {
            id: tenant,
            name: "second tenant".to_owned(),
            slug: format!("second-{tenant}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: athlete.user_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    let token = resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &athlete.user,
            &resources.auth.jwks_manager,
            Some(tenant.to_string()),
        )
        .unwrap();
    (tenant, token)
}

async fn get_json(resources: &Arc<ServerContext>, token: &str, uri: &str) -> (StatusCode, Value) {
    let response = athlete_home_routes()
        .with_state(Arc::clone(resources))
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn recent(resources: &Arc<ServerContext>, token: &str, query: &str) -> Value {
    let (status, body) = get_json(
        resources,
        token,
        &format!("/api/me/activities/recent{query}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

fn ids(body: &Value) -> Vec<String> {
    body["activities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap().to_owned())
        .collect()
}

/// `days` days before a fixed hour today.
fn days_ago(days: i64) -> DateTime<Utc> {
    Utc::now()
        .date_naive()
        .and_hms_opt(6, 0, 0)
        .unwrap()
        .and_utc()
        - Duration::days(days)
}

fn run(id: &str, provider: &str, started: DateTime<Utc>) -> Activity {
    ActivityBuilder::new(
        id,
        format!("Run {id}"),
        SportType::Run,
        started,
        2_400,
        provider,
    )
    .distance_meters(8_000.0)
    .build()
}

async fn cache(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    provider: &str,
    rows: &[Activity],
) {
    cache_in(resources, athlete.user_id, athlete.tenant, provider, rows).await;
}

async fn cache_in(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    provider: &str,
    rows: &[Activity],
) {
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(user_id, &tenant, provider, rows)
        .await
        .unwrap();
}

/// A northbound, weaving 3.3 km track out of Parc La Fontaine.
fn weaving_track(count: usize) -> Vec<(f64, f64)> {
    (0..count)
        .map(|i| {
            let step = i as f64;
            (
                0.0001f64.mul_add(step, 45.5259),
                0.0005f64.mul_add((step * 0.21).sin(), -73.5697),
            )
        })
        .collect()
}

#[tokio::test]
async fn the_newest_five_are_served_across_providers_newest_first() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "recent-order").await;
    let strava: Vec<Activity> = [1, 3, 5, 7]
        .iter()
        .map(|&d| run(&format!("s{d}"), "strava", days_ago(d)))
        .collect();
    let whoop: Vec<Activity> = [2, 4, 6]
        .iter()
        .map(|&d| run(&format!("w{d}"), "whoop", days_ago(d)))
        .collect();
    cache(&resources, &athlete, "strava", &strava).await;
    cache(&resources, &athlete, "whoop", &whoop).await;

    let body = recent(&resources, &athlete.token, "").await;
    assert_eq!(ids(&body), ["s1", "w2", "s3", "w4", "s5"]);
    let providers: Vec<&str> = body["activities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["provider"].as_str().unwrap())
        .collect();
    assert_eq!(providers, ["strava", "whoop", "strava", "whoop", "strava"]);
}

#[tokio::test]
async fn the_limit_is_clamped_between_one_and_twenty() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "recent-clamp").await;
    let rows: Vec<Activity> = (1..=22)
        .map(|d| run(&format!("a{d:02}"), "strava", days_ago(d)))
        .collect();
    cache(&resources, &athlete, "strava", &rows).await;

    assert_eq!(
        ids(&recent(&resources, &athlete.token, "?limit=2").await),
        ["a01", "a02"]
    );
    assert_eq!(
        ids(&recent(&resources, &athlete.token, "?limit=0").await),
        ["a01"]
    );
    assert_eq!(
        ids(&recent(&resources, &athlete.token, "?limit=-4").await),
        ["a01"]
    );
    let widest = ids(&recent(&resources, &athlete.token, "?limit=500").await);
    assert_eq!(widest.len(), 20);
    assert_eq!(widest.first().map(String::as_str), Some("a01"));
    assert_eq!(widest.last().map(String::as_str), Some("a20"));
}

#[tokio::test]
async fn a_row_carries_every_contract_field_with_nulls_where_nothing_was_recorded() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "recent-fields").await;
    let track = weaving_track(300);
    let raw = encode_polyline(&track);
    let started = Utc.with_ymd_and_hms(2026, 9, 20, 7, 30, 0).unwrap();
    let outdoor = ActivityBuilder::new("o1", "Hill loop", SportType::Run, started, 3_000, "strava")
        .distance_meters(9_500.0)
        .elevation_gain(212.0)
        .start_latitude(45.5259)
        .start_longitude(-73.5697)
        .summary_polyline(raw.clone())
        .build();
    let indoor = ActivityBuilder::new(
        "i1",
        "Zwift",
        SportType::VirtualRide,
        started - Duration::days(1),
        3_600,
        "strava",
    )
    .build();
    let gravel = ActivityBuilder::new(
        "g1",
        "Gravel grind",
        SportType::Other("GravelGrind".to_owned()),
        started - Duration::days(2),
        7_200,
        "strava",
    )
    .build();
    cache(&resources, &athlete, "strava", &[outdoor, indoor, gravel]).await;

    let body = recent(&resources, &athlete.token, "").await;
    let rows = body["activities"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    for row in rows {
        let keys: Vec<&str> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        for key in HOME_ACTIVITY_KEYS {
            assert!(keys.contains(&key), "{key} missing from {row}");
        }
        assert_eq!(keys.len(), HOME_ACTIVITY_KEYS.len(), "{row}");
    }

    let outdoor = &rows[0];
    assert_eq!(outdoor["id"], "o1");
    assert_eq!(outdoor["provider"], "strava");
    assert_eq!(outdoor["name"], "Hill loop");
    assert_eq!(outdoor["sport_type"], "run");
    assert_eq!(
        DateTime::parse_from_rfc3339(outdoor["start_date"].as_str().unwrap()).unwrap(),
        started
    );
    assert_eq!(outdoor["duration_seconds"], 3_000);
    assert_eq!(outdoor["distance_meters"], 9_500.0);
    assert_eq!(outdoor["elevation_gain_meters"], 212.0);
    assert_eq!(outdoor["has_gps"], true);
    let sketch = outdoor["summary_polyline"].as_str().unwrap();
    assert_eq!(Some(sketch.to_owned()), trimmed_overview_polyline(&raw));
    assert_ne!(
        sketch, raw,
        "the sketch is trimmed at its ends before it leaves"
    );

    let indoor = &rows[1];
    assert_eq!(indoor["sport_type"], "virtual_ride");
    assert_eq!(
        indoor["has_gps"], true,
        "neither the sport nor a bare row settles it: no route read is stored"
    );
    assert!(indoor["distance_meters"].is_null());
    assert!(indoor["elevation_gain_meters"].is_null());
    assert!(indoor["summary_polyline"].is_null());

    assert_eq!(
        rows[2]["sport_type"], "GravelGrind",
        "an unmapped sport reads as the provider's own word"
    );
}

#[tokio::test]
async fn another_tenants_and_another_users_activities_are_invisible() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "recent-home").await;
    let stranger = seed_athlete(&resources, "recent-stranger").await;
    let (elsewhere, elsewhere_token) = second_tenant(&resources, &athlete).await;
    cache(
        &resources,
        &athlete,
        "strava",
        &[
            run("mine-1", "strava", days_ago(1)),
            run("mine-2", "strava", days_ago(2)),
        ],
    )
    .await;
    cache_in(
        &resources,
        athlete.user_id,
        elsewhere,
        "strava",
        &[run("elsewhere-1", "strava", days_ago(1))],
    )
    .await;
    cache(
        &resources,
        &stranger,
        "strava",
        &[run("theirs-1", "strava", days_ago(1))],
    )
    .await;

    assert_eq!(
        ids(&recent(&resources, &athlete.token, "").await),
        ["mine-1", "mine-2"]
    );
    assert_eq!(
        ids(&recent(&resources, &elsewhere_token, "").await),
        ["elsewhere-1"]
    );
    assert_eq!(
        ids(&recent(&resources, &stranger.token, "").await),
        ["theirs-1"]
    );
}

#[tokio::test]
async fn a_mirror_backend_row_reads_as_the_provider_it_mirrors() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "recent-mirror").await;
    cache(
        &resources,
        &athlete,
        "sciotte",
        &[run("m1", "sciotte", days_ago(1))],
    )
    .await;

    let body = recent(&resources, &athlete.token, "").await;
    assert_eq!(body["activities"][0]["provider"], "strava");
}

/// intervals.icu's API terms require the Garmin attribution beside anything a
/// Garmin device recorded (carnet#521): the row says so, and only that row.
#[tokio::test]
async fn a_garmin_sourced_row_carries_the_garmin_attribution() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "recent-garmin-attribution").await;
    let garmin_recorded = ActivityBuilder::new(
        "g1",
        "Run g1",
        SportType::Run,
        days_ago(1),
        2_400,
        "intervals_icu",
    )
    .source("garmin")
    .build();
    cache(
        &resources,
        &athlete,
        "intervals_icu",
        &[garmin_recorded, run("u2", "intervals_icu", days_ago(2))],
    )
    .await;

    let body = recent(&resources, &athlete.token, "").await;
    assert_eq!(ids(&body), ["g1", "u2"]);
    assert_eq!(body["activities"][0]["attribution"], "Garmin");
    assert!(body["activities"][1]["attribution"].is_null(), "{body}");
}

// ---------------------------------------------------------------------------
// One row per workout: copies across providers merge as chat merges them
// ---------------------------------------------------------------------------

/// One ride a watch recorded and synced to Strava, Garmin and COROS, as each
/// connection caches it: `(cache key, copy)`. Starts sit seconds apart,
/// durations and distances within a percent. The Garmin copy is the longest,
/// so it is the one the session merger keeps; it alone carries the route
/// overview and lacks the elevation the Strava copy recorded.
fn synced_ride(tag: &str, started: DateTime<Utc>) -> [(&'static str, Activity); 3] {
    let overview = encode_polyline(&weaving_track(300));
    [
        (
            oauth_providers::STRAVA,
            ActivityBuilder::new(
                format!("s-{tag}"),
                format!("Sortie {tag}"),
                SportType::Ride,
                started + Duration::seconds(30),
                5_400,
                oauth_providers::STRAVA,
            )
            .distance_meters(41_800.0)
            .elevation_gain(380.0)
            .build(),
        ),
        (
            oauth_providers::SCIOTTE_GARMIN,
            ActivityBuilder::new(
                format!("g-{tag}"),
                format!("Sortie {tag}"),
                SportType::Ride,
                started,
                5_460,
                oauth_providers::SCIOTTE,
            )
            .distance_meters(42_000.0)
            .summary_polyline(overview)
            .build(),
        ),
        (
            oauth_providers::SCIOTTE_COROS,
            ActivityBuilder::new(
                format!("c-{tag}"),
                format!("Sortie {tag}"),
                SportType::Ride,
                started + Duration::seconds(50),
                5_430,
                oauth_providers::SCIOTTE,
            )
            .distance_meters(42_150.0)
            .build(),
        ),
    ]
}

async fn cache_synced(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    copies: &[(&str, Activity)],
) {
    for (key, copy) in copies {
        cache(resources, athlete, key, slice::from_ref(copy)).await;
    }
}

/// `(id, provider)` of every row, in the order the list serves them.
fn ids_and_providers(body: &Value) -> Vec<(String, String)> {
    body["activities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["id"].as_str().unwrap().to_owned(),
                row["provider"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[tokio::test]
async fn one_ride_synced_to_three_providers_is_one_row_the_route_endpoint_serves() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "recent-synced").await;
    cache_synced(&resources, &athlete, &synced_ride("1", days_ago(1))).await;
    cache(
        &resources,
        &athlete,
        oauth_providers::STRAVA,
        &[run("older", oauth_providers::STRAVA, days_ago(2))],
    )
    .await;

    let body = recent(&resources, &athlete.token, "").await;
    assert_eq!(
        ids_and_providers(&body),
        [
            ("g-1".to_owned(), "garmin".to_owned()),
            ("older".to_owned(), "strava".to_owned()),
        ],
        "three copies of one ride are one row, the copy chat keeps: {body}"
    );
    let ride = &body["activities"][0];
    assert_eq!(ride["duration_seconds"], 5_460);
    assert_eq!(ride["distance_meters"], 42_000.0);
    assert_eq!(
        ride["elevation_gain_meters"], 380.0,
        "the merged row carries what the Strava copy recorded"
    );
    assert!(ride["summary_polyline"].is_string(), "{ride}");

    let (status, route) = get_json(
        &resources,
        &athlete.token,
        "/api/me/activities/garmin/g-1/route",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{route}");
    assert_eq!(route["route"]["source_tool"], "garmin");
    assert!(route["reason"].is_null(), "{route}");
}

#[tokio::test]
async fn two_different_workouts_the_same_morning_both_appear() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "recent-same-morning").await;
    let dawn = days_ago(1);
    // A 40-minute run on the watch, then a ride on Strava twenty minutes
    // after the run ended, then an evening run of the same distance.
    cache(
        &resources,
        &athlete,
        oauth_providers::SCIOTTE_GARMIN,
        &[run("dawn-run", oauth_providers::SCIOTTE, dawn)],
    )
    .await;
    cache(
        &resources,
        &athlete,
        oauth_providers::STRAVA,
        &[
            ActivityBuilder::new(
                "commute",
                "Commute",
                SportType::Ride,
                dawn + Duration::minutes(60),
                1_800,
                oauth_providers::STRAVA,
            )
            .distance_meters(12_000.0)
            .build(),
            run(
                "evening-run",
                oauth_providers::STRAVA,
                dawn + Duration::hours(12),
            ),
        ],
    )
    .await;

    let body = recent(&resources, &athlete.token, "").await;
    assert_eq!(
        ids_and_providers(&body),
        [
            ("evening-run".to_owned(), "strava".to_owned()),
            ("commute".to_owned(), "strava".to_owned()),
            ("dawn-run".to_owned(), "garmin".to_owned()),
        ]
    );
}

#[tokio::test]
async fn a_limit_of_five_is_filled_when_copies_hold_the_newest_rows() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "recent-fill").await;
    for provider in [
        oauth_providers::STRAVA,
        oauth_providers::SCIOTTE_GARMIN,
        oauth_providers::SCIOTTE_COROS,
    ] {
        resources
            .common
            .repos
            .provider_connections
            .register_connection(
                athlete.user_id,
                athlete.tenant,
                provider,
                &ConnectionType::OAuth,
                None,
            )
            .await
            .unwrap();
    }
    // Fetched a moment ago, so no background refresh is started.
    resources
        .common
        .repos
        .activity_cache
        .record_activity_fetch(
            athlete.user_id,
            &athlete.tenant,
            oauth_providers::STRAVA,
            Utc::now(),
        )
        .await
        .unwrap();
    // The three newest workouts are each cached three times: nine rows ahead
    // of the four single-copy runs behind them.
    for day in 1..=3 {
        cache_synced(
            &resources,
            &athlete,
            &synced_ride(&day.to_string(), days_ago(day)),
        )
        .await;
    }
    let older: Vec<Activity> = (4..=7)
        .map(|d| run(&format!("o{d}"), oauth_providers::STRAVA, days_ago(d)))
        .collect();
    cache(&resources, &athlete, oauth_providers::STRAVA, &older).await;

    let body = recent(&resources, &athlete.token, "?limit=5").await;
    assert_eq!(body["stale"], false);
    assert_eq!(ids(&body), ["g-1", "g-2", "g-3", "o4", "o5"]);
}

#[tokio::test]
async fn a_limit_of_five_is_filled_when_one_connection_split_the_newest_workout() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "recent-split").await;
    resources
        .common
        .repos
        .provider_connections
        .register_connection(
            athlete.user_id,
            athlete.tenant,
            oauth_providers::STRAVA,
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
    resources
        .common
        .repos
        .activity_cache
        .record_activity_fetch(
            athlete.user_id,
            &athlete.tenant,
            oauth_providers::STRAVA,
            Utc::now(),
        )
        .await
        .unwrap();
    // One connection, so the read makes room for one copy per workout; the
    // newest workout was auto-split into two rows three minutes apart, the
    // two merge into one, and five older runs sit behind them.
    let split_start = Utc::now() - Duration::hours(3);
    let mut rows = vec![
        run("split-a", oauth_providers::STRAVA, split_start),
        run(
            "split-b",
            oauth_providers::STRAVA,
            split_start + Duration::minutes(43),
        ),
    ];
    rows.extend((1..=5).map(|d| run(&format!("o{d}"), oauth_providers::STRAVA, days_ago(d))));
    cache(&resources, &athlete, oauth_providers::STRAVA, &rows).await;

    let body = recent(&resources, &athlete.token, "?limit=5").await;
    assert_eq!(body["stale"], false);
    let shown = ids(&body);
    assert_eq!(shown.len(), 5, "{shown:?}");
    assert_eq!(&shown[1..], ["o1", "o2", "o3", "o4"]);
    assert!(
        shown[0] == "split-a" || shown[0] == "split-b",
        "the split workout is one row on top: {shown:?}"
    );
}

// ---------------------------------------------------------------------------
// has_gps: what the stored route read says, never what the list row lacks
// ---------------------------------------------------------------------------

/// A ride as a mirror backend's list caches it: the numbers of the ride, no
/// start position and no route overview, whether or not it recorded GPS.
fn mirror_ride(id: &str, started: DateTime<Utc>) -> Activity {
    ActivityBuilder::new(
        id,
        format!("Sortie {id}"),
        SportType::Ride,
        started,
        5_400,
        oauth_providers::SCIOTTE,
    )
    .distance_meters(42_000.0)
    .elevation_gain(380.0)
    .average_heart_rate(141)
    .build()
}

/// The key one stored route read is filed under.
struct RouteKey<'a> {
    user_id: Uuid,
    tenant: TenantId,
    provider: &'a str,
    activity_id: &'a str,
}

impl<'a> RouteKey<'a> {
    const fn of(athlete: &Athlete, provider: &'a str, activity_id: &'a str) -> Self {
        Self {
            user_id: athlete.user_id,
            tenant: athlete.tenant,
            provider,
            activity_id,
        }
    }
}

/// Store what a streams read of one activity's route found, as the route
/// endpoint does once it has read the provider.
async fn store_route_read(
    resources: &Arc<ServerContext>,
    key: RouteKey<'_>,
    outcome: Result<RouteTrack, RouteTrackError>,
) {
    let source = RouteGeometrySource::Streams.as_str().to_owned();
    let stored = match outcome {
        Ok(track) => StoredRouteTrack::Drawn {
            source,
            track_json: serde_json::to_string(&track).unwrap(),
        },
        Err(reason) => StoredRouteTrack::Unavailable {
            source,
            reason: reason.as_str().to_owned(),
            expires_at: None,
        },
    };
    resources
        .common
        .repos
        .activity_route_tracks
        .upsert_route_track(
            &key.tenant,
            key.user_id,
            key.provider,
            key.activity_id,
            &stored,
        )
        .await
        .unwrap();
}

/// Store a `no_gps` answer that stands only until `expires_at`, as the route
/// endpoint does when the detail read behind it carried no stream set.
async fn store_unproven_no_gps(
    resources: &Arc<ServerContext>,
    key: RouteKey<'_>,
    expires_at: DateTime<Utc>,
) {
    resources
        .common
        .repos
        .activity_route_tracks
        .upsert_route_track(
            &key.tenant,
            key.user_id,
            key.provider,
            key.activity_id,
            &StoredRouteTrack::Unavailable {
                source: RouteGeometrySource::Streams.as_str().to_owned(),
                reason: RouteTrackError::NoGps.as_str().to_owned(),
                expires_at: Some(expires_at),
            },
        )
        .await
        .unwrap();
}

/// A drawn track, as a streams read of a real ride stores it.
fn drawn_track() -> RouteTrack {
    RouteTrack::from_overview(&weaving_track(300)).unwrap()
}

/// Each row's `(id, has_gps)`, in the order the list serves them.
fn gps_by_id(body: &Value) -> Vec<(String, bool)> {
    body["activities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["id"].as_str().unwrap().to_owned(),
                row["has_gps"].as_bool().unwrap(),
            )
        })
        .collect()
}

fn gps(rows: &[(&str, bool)]) -> Vec<(String, bool)> {
    rows.iter()
        .map(|&(id, has_gps)| (id.to_owned(), has_gps))
        .collect()
}

#[tokio::test]
async fn a_mirror_row_whose_route_was_never_read_says_it_may_have_gps() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "gps-unread").await;
    for (day, backend, id) in [
        (1, oauth_providers::SCIOTTE_GARMIN, "garmin-ride"),
        (2, oauth_providers::SCIOTTE, "strava-ride"),
        (3, oauth_providers::SCIOTTE_COROS, "coros-ride"),
    ] {
        cache(
            &resources,
            &athlete,
            backend,
            &[mirror_ride(id, days_ago(day))],
        )
        .await;
    }

    let body = recent(&resources, &athlete.token, "").await;
    let rows = body["activities"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    for (row, (id, provider)) in rows.iter().zip([
        ("garmin-ride", "garmin"),
        ("strava-ride", "strava"),
        ("coros-ride", "coros"),
    ]) {
        assert_eq!(row["id"], id);
        assert_eq!(row["provider"], provider);
        assert_eq!(row["distance_meters"], 42_000.0);
        assert!(row["summary_polyline"].is_null(), "{row}");
        assert_eq!(
            row["has_gps"], true,
            "a list row that carries no position is not a ride without GPS: {row}"
        );
    }
}

#[tokio::test]
async fn has_gps_is_false_only_for_the_row_whose_stored_read_found_no_gps() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "gps-stored").await;
    let backend = oauth_providers::SCIOTTE_GARMIN;
    let rides: Vec<Activity> = ["unread", "trainer", "drawn", "doorstep"]
        .iter()
        .zip(1..)
        .map(|(id, day)| mirror_ride(id, days_ago(day)))
        .collect();
    cache(&resources, &athlete, backend, &rides).await;

    assert_eq!(
        gps_by_id(&recent(&resources, &athlete.token, "").await),
        gps(&[
            ("unread", true),
            ("trainer", true),
            ("drawn", true),
            ("doorstep", true)
        ]),
        "nothing is stored, so nothing is known to lack GPS"
    );

    store_route_read(
        &resources,
        RouteKey::of(&athlete, backend, "trainer"),
        Err(RouteTrackError::NoGps),
    )
    .await;
    store_route_read(
        &resources,
        RouteKey::of(&athlete, backend, "drawn"),
        Ok(drawn_track()),
    )
    .await;
    store_route_read(
        &resources,
        RouteKey::of(&athlete, backend, "doorstep"),
        Err(RouteTrackError::TooShort),
    )
    .await;
    // The same activity id, read under another provider's key: another
    // provider's activity, which says nothing about this one.
    store_route_read(
        &resources,
        RouteKey::of(&athlete, oauth_providers::SCIOTTE_COROS, "unread"),
        Err(RouteTrackError::NoGps),
    )
    .await;

    let body = recent(&resources, &athlete.token, "").await;
    assert_eq!(
        gps_by_id(&body),
        gps(&[
            ("unread", true),
            ("trainer", false),
            ("drawn", true),
            ("doorstep", true)
        ])
    );
    for row in body["activities"].as_array().unwrap() {
        assert_eq!(row["provider"], "garmin");
        assert!(
            row["summary_polyline"].is_null(),
            "a stored read adds no overview to the row: {row}"
        );
    }
}

#[tokio::test]
async fn a_stored_read_that_is_replaced_changes_what_the_row_says() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "gps-replaced").await;
    let backend = oauth_providers::SCIOTTE;
    cache(
        &resources,
        &athlete,
        backend,
        &[mirror_ride("ride", days_ago(1))],
    )
    .await;

    for (outcome, has_gps) in [
        (Err(RouteTrackError::NoGps), false),
        (Err(RouteTrackError::TooShort), true),
        (Ok(drawn_track()), true),
        (Err(RouteTrackError::NoGps), false),
    ] {
        let stored = outcome.as_ref().map(|_| "drawn").map_err(|e| e.as_str());
        store_route_read(&resources, RouteKey::of(&athlete, backend, "ride"), outcome).await;
        assert_eq!(
            gps_by_id(&recent(&resources, &athlete.token, "").await),
            gps(&[("ride", has_gps)]),
            "after a stored {stored:?}"
        );
    }
}

#[tokio::test]
async fn an_unproven_no_gps_read_says_so_only_until_it_expires() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "gps-unproven").await;
    let backend = oauth_providers::SCIOTTE;
    let rides: Vec<Activity> = ["scraped", "trainer"]
        .iter()
        .zip(1..)
        .map(|(id, day)| mirror_ride(id, days_ago(day)))
        .collect();
    cache(&resources, &athlete, backend, &rides).await;
    // The provider's own word that the trainer ride recorded no GPS: it stands.
    store_route_read(
        &resources,
        RouteKey::of(&athlete, backend, "trainer"),
        Err(RouteTrackError::NoGps),
    )
    .await;

    store_unproven_no_gps(
        &resources,
        RouteKey::of(&athlete, backend, "scraped"),
        Utc::now() + Duration::hours(1),
    )
    .await;
    assert_eq!(
        gps_by_id(&recent(&resources, &athlete.token, "").await),
        gps(&[("scraped", false), ("trainer", false)]),
        "before it expires, the unproven read is the stored answer"
    );

    store_unproven_no_gps(
        &resources,
        RouteKey::of(&athlete, backend, "scraped"),
        Utc::now() - Duration::minutes(1),
    )
    .await;
    assert_eq!(
        gps_by_id(&recent(&resources, &athlete.token, "").await),
        gps(&[("scraped", true), ("trainer", false)]),
        "an expired read is no read: the client asks for the route again"
    );
    let repo = &resources.common.repos.activity_route_tracks;
    assert_eq!(
        repo.get_route_track(&athlete.tenant, athlete.user_id, backend, "scraped")
            .await
            .unwrap(),
        None,
        "the route read finds no stored answer either"
    );
    assert_eq!(
        repo.get_route_track(&athlete.tenant, athlete.user_id, backend, "trainer")
            .await
            .unwrap(),
        Some(StoredRouteTrack::Unavailable {
            source: "streams".to_owned(),
            reason: "no_gps".to_owned(),
            expires_at: None,
        })
    );
}

#[tokio::test]
async fn a_route_read_stored_for_another_tenant_or_user_never_reaches_the_row() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "gps-tenancy").await;
    let stranger = seed_athlete(&resources, "gps-stranger").await;
    let (elsewhere, elsewhere_token) = second_tenant(&resources, &athlete).await;
    let backend = oauth_providers::SCIOTTE_GARMIN;
    let rides = [mirror_ride("same-id", days_ago(1))];
    cache(&resources, &athlete, backend, &rides).await;
    cache_in(&resources, athlete.user_id, elsewhere, backend, &rides).await;
    cache(&resources, &stranger, backend, &rides).await;

    // The same user, provider key and activity id — in the athlete's other
    // tenant.
    store_route_read(
        &resources,
        RouteKey {
            tenant: elsewhere,
            ..RouteKey::of(&athlete, backend, "same-id")
        },
        Err(RouteTrackError::NoGps),
    )
    .await;
    // The same tenant, provider key and activity id — for another user.
    store_route_read(
        &resources,
        RouteKey {
            user_id: stranger.user_id,
            ..RouteKey::of(&athlete, backend, "same-id")
        },
        Err(RouteTrackError::NoGps),
    )
    .await;

    assert_eq!(
        gps_by_id(&recent(&resources, &athlete.token, "").await),
        gps(&[("same-id", true)]),
        "neither read is this tenant's read of this athlete's ride"
    );
    assert_eq!(
        gps_by_id(&recent(&resources, &elsewhere_token, "").await),
        gps(&[("same-id", false)]),
        "the other tenant reads its own"
    );
    assert_eq!(
        gps_by_id(&recent(&resources, &stranger.token, "").await),
        gps(&[("same-id", true)]),
        "the stranger's own tenant holds no read of their ride"
    );
}

// ---------------------------------------------------------------------------
// A merged workout draws the route of a copy that has one (carnet#674)
// ---------------------------------------------------------------------------

/// The 2026-09-28 run as jf@dravr.ai's Strava mirror caches it: a manual
/// entry typed in at 11:45:00 sharp with the run's 12.83 km and no sensor,
/// and the watch's recording of the same run started at 12:43:54. The manual
/// entry is the longer, so the session merger keeps it for the numbers.
fn manual_and_recorded_run(day: DateTime<Utc>) -> [Activity; 2] {
    let at = |h, m, s| day.date_naive().and_hms_opt(h, m, s).unwrap().and_utc();
    [
        ActivityBuilder::new(
            "bug-de-fougeres",
            "Bug de Fougères",
            SportType::Run,
            at(11, 45, 0),
            3_900,
            oauth_providers::SCIOTTE,
        )
        .distance_meters(12_830.0)
        .build(),
        ActivityBuilder::new(
            "morning-trail-run",
            "Morning Trail Run",
            SportType::Run,
            at(12, 43, 54),
            3_480,
            oauth_providers::SCIOTTE,
        )
        .distance_meters(12_910.0)
        .elevation_gain(214.0)
        .average_heart_rate(152)
        .build(),
    ]
}

#[tokio::test]
async fn a_manual_copy_merged_with_a_gps_recording_draws_the_recordings_route() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "merged-route").await;
    cache(
        &resources,
        &athlete,
        oauth_providers::SCIOTTE,
        &manual_and_recorded_run(days_ago(2)),
    )
    .await;

    let assert_row = |body: &Value| {
        let rows = body["activities"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "the two copies are one workout: {body}");
        let row = &rows[0];
        assert_eq!(
            row["id"], "morning-trail-run",
            "the row reads its route from the GPS recording: {row}"
        );
        assert_eq!(row["provider"], "strava");
        assert_eq!(row["has_gps"], true, "{row}");
        assert_eq!(row["name"], "Bug de Fougères", "the canonical copy's name");
        assert_eq!(row["distance_meters"], 12_830.0, "the canonical numbers");
        assert_eq!(row["duration_seconds"], 3_900);
        assert_eq!(
            row["elevation_gain_meters"], 214.0,
            "what the canonical copy lacked, from the recording"
        );
    };

    // Before any route read: the recording carries a heart rate, the manual
    // entry nothing a device records.
    assert_row(&recent(&resources, &athlete.token, "").await);

    // The incident: the manual copy's read found no GPS.
    store_route_read(
        &resources,
        RouteKey::of(&athlete, oauth_providers::SCIOTTE, "bug-de-fougeres"),
        Err(RouteTrackError::NoGps),
    )
    .await;
    assert_row(&recent(&resources, &athlete.token, "").await);

    // The recording's route, once read, is what the row's route endpoint
    // serves.
    store_route_read(
        &resources,
        RouteKey::of(&athlete, oauth_providers::SCIOTTE, "morning-trail-run"),
        Ok(drawn_track()),
    )
    .await;
    assert_row(&recent(&resources, &athlete.token, "").await);
    let (status, route) = get_json(
        &resources,
        &athlete.token,
        "/api/me/activities/strava/morning-trail-run/route",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{route}");
    assert!(route["reason"].is_null(), "{route}");
    assert_eq!(
        route["route"]["coordinates"].as_array().unwrap().len(),
        drawn_track().coordinates.len(),
        "{route}"
    );
}

#[tokio::test]
async fn a_merged_workout_keeps_its_canonical_copy_when_no_copy_says_more() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "merged-canonical").await;
    let [manual, recorded] = manual_and_recorded_run(days_ago(2));
    // The recording as a list that carries no sensor data caches it.
    let bare = ActivityBuilder::new(
        recorded.id(),
        recorded.name(),
        SportType::Run,
        recorded.start_date(),
        recorded.duration_seconds(),
        oauth_providers::SCIOTTE,
    )
    .distance_meters(12_910.0)
    .build();
    cache(
        &resources,
        &athlete,
        oauth_providers::SCIOTTE,
        &[manual, bare],
    )
    .await;

    let body = recent(&resources, &athlete.token, "").await;
    assert_eq!(
        gps_by_id(&body),
        gps(&[("bug-de-fougeres", true)]),
        "nothing tells the copies apart: the merger's canonical copy: {body}"
    );

    // Its read found no GPS: the other copy may still hold a track.
    store_route_read(
        &resources,
        RouteKey::of(&athlete, oauth_providers::SCIOTTE, "bug-de-fougeres"),
        Err(RouteTrackError::NoGps),
    )
    .await;
    let body = recent(&resources, &athlete.token, "").await;
    assert_eq!(
        gps_by_id(&body),
        gps(&[("morning-trail-run", true)]),
        "a copy whose read found no GPS gives way to one never read: {body}"
    );

    // Both read, neither drew: the row says so.
    store_route_read(
        &resources,
        RouteKey::of(&athlete, oauth_providers::SCIOTTE, "morning-trail-run"),
        Err(RouteTrackError::NoGps),
    )
    .await;
    let body = recent(&resources, &athlete.token, "").await;
    assert_eq!(
        gps_by_id(&body),
        gps(&[("bug-de-fougeres", false)]),
        "no copy recorded GPS: the canonical copy, without GPS: {body}"
    );
}

#[tokio::test]
async fn freshness_follows_the_last_successful_fetch() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "recent-fresh").await;

    // Nothing connected, nothing fetched: nothing to refresh either.
    let body = recent(&resources, &athlete.token, "").await;
    assert!(body["as_of"].is_null());
    assert_eq!(body["stale"], false);
    assert_eq!(body["activities"], json!([]));

    // A fetch a moment ago is fresh, and `as_of` is when it happened.
    let fetched_at = Utc::now() - Duration::minutes(5);
    resources
        .common
        .repos
        .provider_connections
        .register_connection(
            athlete.user_id,
            athlete.tenant,
            "whoop",
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
    resources
        .common
        .repos
        .activity_cache
        .record_activity_fetch(athlete.user_id, &athlete.tenant, "whoop", fetched_at)
        .await
        .unwrap();
    let body = recent(&resources, &athlete.token, "").await;
    assert_eq!(body["stale"], false);
    let as_of = DateTime::parse_from_rfc3339(body["as_of"].as_str().unwrap()).unwrap();
    assert!((as_of.with_timezone(&Utc) - fetched_at).num_seconds().abs() < 1);
}

#[tokio::test]
async fn a_connected_provider_never_fetched_is_stale() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "recent-never").await;
    // A connection with no token: the background refresh finds nothing to
    // authenticate with and leaves the cache as it was.
    resources
        .common
        .repos
        .provider_connections
        .register_connection(
            athlete.user_id,
            athlete.tenant,
            "whoop",
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();

    let body = recent(&resources, &athlete.token, "").await;
    assert!(body["as_of"].is_null());
    assert_eq!(body["stale"], true);
    await_background(&resources).await;
}

// ---------------------------------------------------------------------------
// The background refresh, against a Strava-shaped mock
// ---------------------------------------------------------------------------

/// Environment set for the duration of one test and removed after it.
struct EnvGuard {
    keys: Vec<&'static str>,
}

impl EnvGuard {
    fn set(vars: &[(&'static str, String)]) -> Self {
        for (key, value) in vars {
            env::set_var(key, value);
        }
        Self {
            keys: vars.iter().map(|(key, _)| *key).collect(),
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for key in &self.keys {
            env::remove_var(key);
        }
    }
}

/// A mock `/athlete/activities` answering one fresh outdoor run, with a hit
/// counter.
async fn mock_strava_list() -> (String, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);
    let polyline = encode_polyline(&weaving_track(300));
    let app = Router::new().route(
        "/athlete/activities",
        get(move |_: Query<HashMap<String, String>>| {
            let counter = Arc::clone(&counter);
            let polyline = polyline.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                // Slow enough that concurrent page loads overlap the refresh.
                sleep(StdDuration::from_millis(150)).await;
                Json(json!([{
                    "id": 777_001,
                    "name": "Morning loop",
                    "type": "Run",
                    "sport_type": "Run",
                    "start_date": (Utc::now() - Duration::hours(2)).to_rfc3339(),
                    "distance": 10_200.0,
                    "elapsed_time": 3_100,
                    "total_elevation_gain": 95.0,
                    "start_latlng": [45.5259, -73.5697],
                    "map": { "id": "a777001", "summary_polyline": polyline }
                }]))
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), hits)
}

async fn link_strava(resources: &Arc<ServerContext>, athlete: &Athlete) {
    let repos = &resources.common.repos;
    repos
        .provider_connections
        .register_connection(
            athlete.user_id,
            athlete.tenant,
            "strava",
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
    let now = Utc::now();
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id: athlete.user_id,
            tenant_id: athlete.tenant.to_string(),
            provider: "strava".to_owned(),
            // >= 40 chars and not "at_"-prefixed: the provider's token validation.
            access_token: "strava_access_token_0123456789abcdef0123456789".to_owned(),
            refresh_token: Some("strava_refresh".to_owned()),
            token_type: "Bearer".to_owned(),
            expires_at: Some(now + Duration::hours(6)),
            scope: Some("activity:read_all".to_owned()),
            provider_user_id: Some("4242".to_owned()),
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
}

/// Move every cached row's `synced_at` back to `at`, as if the write-through
/// that stored them happened then. No repository writes an old `synced_at`
/// (an upsert stamps now), so the fixture writes it in SQL.
async fn backdate_cached_rows(resources: &ServerContext, user_id: Uuid, at: DateTime<Utc>) {
    const SQL: &str = "UPDATE cached_activities SET synced_at = $1 WHERE user_id = $2";
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query(SQL)
                .bind(at)
                .bind(user_id.to_string())
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => {
            sqlx::query(SQL)
                .bind(at)
                .bind(user_id.to_string())
                .execute(postgres.pool())
                .await
                .unwrap();
        }
    }
}

/// Wait for every task the route spawned on the drain tracker to finish.
async fn await_background(resources: &ServerContext) {
    let deadline = Instant::now() + StdDuration::from_secs(20);
    while !resources.common.turns.is_empty() {
        assert!(
            Instant::now() < deadline,
            "the background refresh did not finish within 20s"
        );
        sleep(StdDuration::from_millis(25)).await;
    }
}

#[tokio::test]
#[serial]
async fn a_stale_cache_answers_at_once_and_refreshes_once_in_the_background() {
    let (api_base, hits) = mock_strava_list().await;
    let _env = EnvGuard::set(&[
        ("PIERRE_STRAVA_API_BASE_URL", api_base),
        ("STRAVA_CLIENT_ID", "test_client".to_owned()),
        ("STRAVA_CLIENT_SECRET", "test_secret".to_owned()),
    ]);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "recent-refresh").await;
    link_strava(&resources, &athlete).await;
    // What the cache held from a fetch two days ago.
    cache(
        &resources,
        &athlete,
        "strava",
        &[run("old-1", "strava", days_ago(3))],
    )
    .await;
    let two_days_ago = Utc::now() - Duration::days(2);
    backdate_cached_rows(&resources, athlete.user_id, two_days_ago).await;
    resources
        .common
        .repos
        .activity_cache
        .record_activity_fetch(athlete.user_id, &athlete.tenant, "strava", two_days_ago)
        .await
        .unwrap();

    // Several page loads while the cache is stale: each answers from the
    // cache at once, and together they start one provider read.
    let loads = (0..4).map(|_| recent(&resources, &athlete.token, ""));
    for body in join_all(loads).await {
        assert_eq!(body["stale"], true);
        assert_eq!(ids(&body), ["old-1"]);
    }
    await_background(&resources).await;
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "one refresh for every stale load"
    );

    // The refresh wrote through: the next load is fresh and carries the run
    // the provider returned, route overview and all.
    let body = recent(&resources, &athlete.token, "").await;
    assert_eq!(body["stale"], false);
    assert_eq!(ids(&body), ["777001", "old-1"]);
    let fresh = &body["activities"][0];
    assert_eq!(fresh["has_gps"], true);
    assert!(fresh["summary_polyline"]
        .as_str()
        .is_some_and(|p| !p.is_empty()));
    let as_of = DateTime::parse_from_rfc3339(body["as_of"].as_str().unwrap()).unwrap();
    assert!(as_of.with_timezone(&Utc) > two_days_ago + Duration::days(1));
    await_background(&resources).await;
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "a fresh cache reads no provider"
    );
}

/// Each provider is judged by its own last fetch. A WHOOP synced minutes ago
/// says nothing about a Strava last read two days ago: the page is stale, the
/// Strava head is refreshed, and `as_of` still shows the newest fetch of any
/// provider.
#[tokio::test]
#[serial]
async fn a_fresh_provider_does_not_hide_a_stale_one() {
    let (api_base, hits) = mock_strava_list().await;
    let _env = EnvGuard::set(&[
        ("PIERRE_STRAVA_API_BASE_URL", api_base),
        ("STRAVA_CLIENT_ID", "test_client".to_owned()),
        ("STRAVA_CLIENT_SECRET", "test_secret".to_owned()),
    ]);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "recent-per-provider").await;
    link_strava(&resources, &athlete).await;
    let repos = &resources.common.repos;
    repos
        .provider_connections
        .register_connection(
            athlete.user_id,
            athlete.tenant,
            "whoop",
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
    let strava_fetched = Utc::now() - Duration::days(2);
    let whoop_fetched = Utc::now() - Duration::minutes(5);
    repos
        .activity_cache
        .record_activity_fetch(athlete.user_id, &athlete.tenant, "strava", strava_fetched)
        .await
        .unwrap();
    repos
        .activity_cache
        .record_activity_fetch(athlete.user_id, &athlete.tenant, "whoop", whoop_fetched)
        .await
        .unwrap();

    let body = recent(&resources, &athlete.token, "").await;
    assert_eq!(body["stale"], true, "the stale Strava makes the page stale");
    let as_of = DateTime::parse_from_rfc3339(body["as_of"].as_str().unwrap()).unwrap();
    assert!(
        (as_of.with_timezone(&Utc) - whoop_fetched)
            .num_seconds()
            .abs()
            < 1,
        "as_of is the newest fetch of any provider"
    );
    await_background(&resources).await;
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "the stale Strava was refreshed"
    );
}
