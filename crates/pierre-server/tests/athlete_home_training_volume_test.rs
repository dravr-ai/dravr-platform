// ABOUTME: GET /api/me/training-volume — weekly distance, time and climbing per sport over twelve weeks, from the activity cache
// ABOUTME: Pins one count per workout across providers, weeks absent before the stored history begins, tenancy, and the empty answer

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Athlete Home training-volume suite.
//!
//! Every test seeds real rows through the activity cache's own writer and
//! reads the route's JSON back, so a handler that answered fixed weeks, zeros
//! for weeks nobody stored, or another tenant's rows fails on content.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
#![cfg(feature = "provider-strava")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{Activity, ActivityBuilder, SportType, TenantId};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use pierre_mcp_server::services::training_volume::{monday_of, VOLUME_WEEKS};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

struct Athlete {
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
        user_id,
        tenant,
        token,
    }
}

async fn volume_response(
    resources: &Arc<ServerContext>,
    token: Option<&str>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().uri("/api/me/training-volume");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = athlete_home_routes()
        .with_state(Arc::clone(resources))
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn volume(resources: &Arc<ServerContext>, athlete: &Athlete) -> Value {
    let (status, body) = volume_response(resources, Some(&athlete.token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// 06:00 UTC `days` days before today, `days` at least one; the seeded
/// athletes have no timezone, so their calendar is UTC's.
fn days_ago(days: i64) -> DateTime<Utc> {
    Utc::now()
        .date_naive()
        .and_hms_opt(6, 0, 0)
        .unwrap()
        .and_utc()
        - Duration::days(days)
}

/// `hours` hours before now: always in the past, so the read up to now holds it.
fn hours_ago(hours: i64) -> DateTime<Utc> {
    Utc::now() - Duration::hours(hours)
}

/// The Monday of the week an instant falls in, on the UTC calendar.
fn week_of(instant: DateTime<Utc>) -> NaiveDate {
    monday_of(instant.date_naive())
}

fn ymd(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

fn activity(
    id: &str,
    sport: SportType,
    started: DateTime<Utc>,
    seconds: u64,
    provider: &str,
) -> ActivityBuilder {
    ActivityBuilder::new(
        id,
        format!("Session {id}"),
        sport,
        started,
        seconds,
        provider,
    )
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

/// The week entry whose Monday is `week_start`.
fn week(body: &Value, week_start: NaiveDate) -> &Value {
    body["weeks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|week| week["week_start"] == ymd(week_start))
        .unwrap_or_else(|| panic!("no week starting {week_start}: {body}"))
}

/// The `sport` entry of the week `instant` falls in.
fn sport_in<'a>(body: &'a Value, instant: DateTime<Utc>, sport: &str) -> &'a Value {
    week(body, week_of(instant))["sports"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["sport_type"] == sport)
        .unwrap_or_else(|| panic!("no {sport} in the week of {instant}: {body}"))
}

fn week_starts(body: &Value) -> Vec<String> {
    body["weeks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|week| week["week_start"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn a_deep_history_answers_twelve_weeks_summed_per_sport() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "volume-deep").await;
    // An old run puts the history behind the whole window.
    let run_at = hours_ago(3);
    let swim_at = hours_ago(1);
    cache(
        &resources,
        &athlete,
        oauth_providers::STRAVA,
        &[
            activity(
                "old",
                SportType::Run,
                days_ago(100),
                3_000,
                oauth_providers::STRAVA,
            )
            .distance_meters(9_000.0)
            .build(),
            activity("r1", SportType::Run, run_at, 3_600, oauth_providers::STRAVA)
                .distance_meters(10_000.0)
                .elevation_gain(90.0)
                .build(),
            activity(
                "swim",
                SportType::Swim,
                swim_at,
                1_800,
                oauth_providers::STRAVA,
            )
            .distance_meters(1_500.0)
            .build(),
            activity(
                "r2",
                SportType::Run,
                days_ago(21),
                2_400,
                oauth_providers::STRAVA,
            )
            .distance_meters(7_000.0)
            .build(),
        ],
    )
    .await;

    let body = volume(&resources, &athlete).await;
    let today = Utc::now().date_naive();
    assert_eq!(body["today"], ymd(today));
    let starts = week_starts(&body);
    assert_eq!(starts.len(), usize::try_from(VOLUME_WEEKS).unwrap());
    assert_eq!(starts.last().unwrap(), &ymd(monday_of(today)));
    assert_eq!(
        starts.first().unwrap(),
        &ymd(monday_of(today) - Duration::weeks(VOLUME_WEEKS - 1))
    );

    assert_eq!(
        sport_in(&body, run_at, "run"),
        &json!({
            "sport_type": "run",
            "activities": 1,
            "distance_meters": 10_000.0,
            "duration_seconds": 3_600,
            "elevation_gain_meters": 90.0,
        })
    );
    assert_eq!(
        sport_in(&body, swim_at, "swim"),
        &json!({
            "sport_type": "swim",
            "activities": 1,
            "distance_meters": 1_500.0,
            "duration_seconds": 1_800,
            "elevation_gain_meters": 0.0,
        })
    );
    assert_eq!(
        week(&body, monday_of(today - Duration::days(21)))["sports"],
        json!([{
            "sport_type": "run",
            "activities": 1,
            "distance_meters": 7_000.0,
            "duration_seconds": 2_400,
            "elevation_gain_meters": 0.0,
        }])
    );
    // A week inside the vouched-for window with no training is an empty week.
    let quiet = body["weeks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|week| week["sports"].as_array().unwrap().is_empty())
        .count();
    assert!(quiet >= 9, "{body}");
}

#[tokio::test]
async fn one_ride_synced_to_two_providers_counts_once() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "volume-synced").await;
    let started = hours_ago(2);
    cache(
        &resources,
        &athlete,
        oauth_providers::STRAVA,
        &[activity(
            "s-1",
            SportType::Ride,
            started + Duration::seconds(30),
            5_400,
            oauth_providers::STRAVA,
        )
        .distance_meters(41_800.0)
        .elevation_gain(380.0)
        .build()],
    )
    .await;
    cache(
        &resources,
        &athlete,
        oauth_providers::SCIOTTE_GARMIN,
        &[activity(
            "g-1",
            SportType::Ride,
            started,
            5_460,
            oauth_providers::SCIOTTE,
        )
        .distance_meters(42_000.0)
        .build()],
    )
    .await;

    let body = volume(&resources, &athlete).await;
    let sports = &week(&body, week_of(started))["sports"];
    assert_eq!(sports.as_array().unwrap().len(), 1, "{body}");
    let ride = &sports[0];
    assert_eq!(ride["sport_type"], "ride");
    assert_eq!(
        ride["activities"], 1,
        "two copies of one ride are one workout: {body}"
    );
    // The merger keeps the longer copy and fills the climbing it lacked.
    assert_eq!(ride["duration_seconds"], 5_460);
    assert_eq!(ride["distance_meters"], 42_000.0);
    assert_eq!(ride["elevation_gain_meters"], 380.0);
}

#[tokio::test]
async fn weeks_before_the_stored_history_are_absent_not_zero() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "volume-thin").await;
    cache(
        &resources,
        &athlete,
        oauth_providers::STRAVA,
        &[activity(
            "first",
            SportType::Run,
            days_ago(15),
            1_800,
            oauth_providers::STRAVA,
        )
        .distance_meters(5_000.0)
        .build()],
    )
    .await;

    let body = volume(&resources, &athlete).await;
    let today = Utc::now().date_naive();
    let first = monday_of(today - Duration::days(15));
    let mut expected = Vec::new();
    let mut monday = first;
    while monday <= monday_of(today) {
        expected.push(ymd(monday));
        monday += Duration::weeks(1);
    }
    assert_eq!(week_starts(&body), expected);
    assert_eq!(week(&body, first)["sports"][0]["activities"], 1);
}

#[tokio::test]
async fn nothing_stored_answers_no_weeks() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "volume-empty").await;
    let body = volume(&resources, &athlete).await;
    assert_eq!(body["weeks"], json!([]));
}

#[tokio::test]
async fn another_tenants_and_another_users_activities_are_invisible() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "volume-home").await;
    let stranger = seed_athlete(&resources, "volume-stranger").await;
    let elsewhere = TenantId::generate();
    let mine_at = hours_ago(1);
    cache(
        &resources,
        &athlete,
        oauth_providers::STRAVA,
        &[activity(
            "mine",
            SportType::Run,
            mine_at,
            1_200,
            oauth_providers::STRAVA,
        )
        .distance_meters(3_000.0)
        .build()],
    )
    .await;
    cache_in(
        &resources,
        athlete.user_id,
        elsewhere,
        oauth_providers::STRAVA,
        &[activity(
            "elsewhere",
            SportType::Run,
            hours_ago(2),
            9_000,
            oauth_providers::STRAVA,
        )
        .distance_meters(30_000.0)
        .build()],
    )
    .await;
    cache(
        &resources,
        &stranger,
        oauth_providers::STRAVA,
        &[activity(
            "theirs",
            SportType::Run,
            hours_ago(3),
            9_000,
            oauth_providers::STRAVA,
        )
        .distance_meters(30_000.0)
        .build()],
    )
    .await;

    let body = volume(&resources, &athlete).await;
    let sports = &week(&body, week_of(mine_at))["sports"];
    assert_eq!(
        sports,
        &json!([{
            "sport_type": "run",
            "activities": 1,
            "distance_meters": 3_000.0,
            "duration_seconds": 1_200,
            "elevation_gain_meters": 0.0,
        }])
    );
}

#[tokio::test]
async fn an_unauthenticated_read_is_refused() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (status, _) = volume_response(&resources, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
