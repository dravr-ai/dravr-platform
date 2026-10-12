// ABOUTME: The gear ledger over REST — default pairs accrue workouts once, a choice moves distance, the warning flips at its threshold
// ABOUTME: Retire keeps history, delete waits for none, a deleted upload lets its pair go, one athlete never reads another's pairs, every device reaches Home

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Gear ledger suite (carnet#711).
//!
//! Every test seeds real rows through the activity cache's and the ledger's
//! own writers and reads the routes' JSON back, so a handler that answered a
//! fixed distance, counted a workout twice or served another athlete's pair
//! fails on content.
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
use axum::http::{Method, Request, StatusCode};
use chrono::{DateTime, Duration, Utc};
use pierre_core::models::{
    Activity, ActivityBuilder, GearDeclaration, GearKind, SportType, TenantId,
    DEFAULT_SHOE_WARN_DISTANCE_METERS,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_gear::athlete_gear_routes;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use pierre_test_support::fit::{FitSport, FitWorkout};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

/// One athlete: the account, the tenant they act in and a bearer for it.
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

/// Call the gear router as `athlete`, with a JSON body when one is given.
async fn call(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {}", athlete.token));
    let request = match body {
        Some(body) => request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())),
        None => request.body(Body::empty()),
    }
    .unwrap();
    let response = athlete_gear_routes()
        .with_state(Arc::clone(resources))
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// `GET /api/me/gear`, which must answer 200.
async fn ledger(resources: &Arc<ServerContext>, athlete: &Athlete) -> Vec<Value> {
    let (status, body) = call(resources, athlete, Method::GET, "/api/me/gear", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["gear"].as_array().unwrap().clone()
}

/// The pair `id` in the athlete's ledger.
async fn pair(resources: &Arc<ServerContext>, athlete: &Athlete, id: Uuid) -> Value {
    ledger(resources, athlete)
        .await
        .into_iter()
        .find(|pair| pair["id"] == id.to_string())
        .unwrap_or_else(|| panic!("pair {id} in the ledger"))
}

fn meters(pair: &Value) -> f64 {
    pair["distance_meters"].as_f64().unwrap()
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

fn run(id: &str, provider: &str, started: DateTime<Utc>, meters: f64) -> Activity {
    ActivityBuilder::new(
        id,
        format!("Run {id}"),
        SportType::Run,
        started,
        2_400,
        provider,
    )
    .distance_meters(meters)
    .build()
}

async fn cache(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    provider: &str,
    rows: &[Activity],
) {
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(athlete.user_id, &athlete.tenant, provider, rows)
        .await
        .unwrap();
}

/// A road pair declared `added_days_ago` days ago, through the ledger's own
/// writer so its history can start in the past.
async fn declare(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    name: &str,
    initial: f64,
    warn: f64,
    added_days_ago: i64,
) -> Uuid {
    resources
        .common
        .repos
        .athlete_gear
        .create_gear(
            athlete.user_id,
            &GearDeclaration {
                kind: GearKind::Shoe,
                name: name.to_owned(),
                sport: SportType::Run,
                initial_distance_meters: initial,
                warn_distance_meters: warn,
                pinned: false,
            },
            days_ago(added_days_ago),
        )
        .await
        .unwrap()
}

/// The same morning run, recorded once and synced to Strava and to
/// intervals.icu: two cached copies of one workout.
async fn cache_one_run_twice(resources: &Arc<ServerContext>, athlete: &Athlete, meters: f64) {
    let start = days_ago(2);
    cache(
        resources,
        athlete,
        "strava",
        &[run("s1", "strava", start, meters)],
    )
    .await;
    cache(
        resources,
        athlete,
        "intervals_icu",
        &[run(
            "i1",
            "intervals_icu",
            start + Duration::seconds(30),
            meters,
        )],
    )
    .await;
}

/// Call the Home router as `athlete`.
async fn home(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    method: Method,
    uri: &str,
    body: Body,
) -> (StatusCode, Value) {
    let response = athlete_home_routes()
        .with_state(Arc::clone(resources))
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", format!("Bearer {}", athlete.token))
                .header("content-type", "application/octet-stream")
                .body(body)
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

/// A ten-minute run's FIT file, recorded from `start`.
fn fit_run(start: DateTime<Utc>) -> FitWorkout {
    FitWorkout {
        start,
        seconds: 600,
        sport: FitSport::Running,
        with_position: true,
    }
}

/// Upload `workout` through the Home route, answering the activity id it
/// became.
async fn upload(resources: &Arc<ServerContext>, athlete: &Athlete, workout: &FitWorkout) -> String {
    let (status, body) = home(
        resources,
        athlete,
        Method::POST,
        "/api/me/activities/upload",
        Body::from(workout.encode()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["activities"][0]["id"].as_str().unwrap().to_owned()
}

/// Delete the athlete's uploaded activity `id`, which must answer 204.
async fn delete_upload(resources: &Arc<ServerContext>, athlete: &Athlete, id: &str) {
    let (status, body) = home(
        resources,
        athlete,
        Method::DELETE,
        &format!("/api/me/activities/upload/{id}"),
        Body::empty(),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
}

#[tokio::test]
async fn a_declared_pair_answers_with_its_shape_and_the_default_warning() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "gear-create").await;

    let (status, created) = call(
        &resources,
        &athlete,
        Method::POST,
        "/api/me/gear",
        Some(
            json!({"name": "  Pegasus 41 ", "sport": "run", "initial_distance_meters": 120_000.0}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["name"], "Pegasus 41");
    assert_eq!(created["kind"], "shoe");
    assert_eq!(created["sport"], "run");
    assert_eq!(
        created["warn_distance_meters"].as_f64(),
        Some(DEFAULT_SHOE_WARN_DISTANCE_METERS)
    );
    assert_eq!(created["distance_meters"].as_f64(), Some(120_000.0));
    assert_eq!(created["activity_count"], 0);
    assert_eq!(created["warn_reached"], false);
    assert_eq!(
        created["is_default"], true,
        "the only run pair is the default"
    );
    assert!(created["retired_at"].is_null());
    assert!(created["last_used_at"].is_null());

    for refused in [
        json!({"name": "Bike shoes", "sport": "ride"}),
        json!({"name": "   ", "sport": "run"}),
        json!({"name": "Pegasus", "sport": "run", "warn_distance_meters": 0.0}),
        json!({"name": "Pegasus", "sport": "run", "initial_distance_meters": -5.0}),
    ] {
        let (status, body) = call(
            &resources,
            &athlete,
            Method::POST,
            "/api/me/gear",
            Some(refused.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{refused} → {body}");
    }
    assert_eq!(ledger(&resources, &athlete).await.len(), 1);
}

#[tokio::test]
async fn a_new_run_accrues_to_the_default_pair_once_whichever_providers_hold_it() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "gear-accrue").await;
    let id = declare(&resources, &athlete, "Daily", 0.0, 500_000.0, 10).await;
    // A run from before the pair was declared stays out of it.
    cache(
        &resources,
        &athlete,
        "strava",
        &[run("old", "strava", days_ago(20), 15_000.0)],
    )
    .await;
    cache_one_run_twice(&resources, &athlete, 10_000.0).await;

    let daily = pair(&resources, &athlete, id).await;
    assert!((meters(&daily) - 10_000.0).abs() < 1e-6, "{daily}");
    assert_eq!(
        daily["activity_count"], 1,
        "two copies, one workout: {daily}"
    );
    assert_eq!(daily["is_default"], true);
    assert!(daily["last_used_at"].is_string());

    // Reading again adds nothing: the default was recorded once.
    let again = pair(&resources, &athlete, id).await;
    assert!((meters(&again) - 10_000.0).abs() < 1e-6, "{again}");

    // Both copies answer with the same default.
    for path in [
        "/api/me/activities/strava/s1/gear",
        "/api/me/activities/intervals_icu/i1/gear",
    ] {
        let (status, body) = call(&resources, &athlete, Method::GET, path, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["gear_id"], id.to_string(), "{path}");
        assert_eq!(body["source"], "default", "{path}");
    }
    let (status, body) = call(
        &resources,
        &athlete,
        Method::GET,
        "/api/me/activities/strava/old/gear",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["gear_id"].is_null() && body["source"].is_null(),
        "{body}"
    );
}

#[tokio::test]
async fn choosing_another_pair_moves_the_workouts_distance_and_none_stays_none() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "gear-reassign").await;
    let road = declare(&resources, &athlete, "Road", 0.0, 500_000.0, 10).await;
    let spare = declare(&resources, &athlete, "Spare", 0.0, 500_000.0, 12).await;
    cache_one_run_twice(&resources, &athlete, 10_000.0).await;
    assert!((meters(&pair(&resources, &athlete, road).await) - 10_000.0).abs() < 1e-6);

    // Chosen on the Strava copy; the default sat on the intervals.icu copy.
    let (status, body) = call(
        &resources,
        &athlete,
        Method::PUT,
        "/api/me/activities/strava/s1/gear",
        Some(json!({"gear_id": spare.to_string()})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["gear_id"], spare.to_string());
    assert_eq!(body["source"], "declared");

    let road_now = pair(&resources, &athlete, road).await;
    let spare_now = pair(&resources, &athlete, spare).await;
    assert!(meters(&road_now).abs() < 1e-6, "{road_now}");
    assert_eq!(road_now["activity_count"], 0);
    assert!((meters(&spare_now) - 10_000.0).abs() < 1e-6, "{spare_now}");
    assert_eq!(
        spare_now["is_default"], true,
        "the pair used last is the default"
    );
    let (_, other_copy) = call(
        &resources,
        &athlete,
        Method::GET,
        "/api/me/activities/intervals_icu/i1/gear",
        None,
    )
    .await;
    assert_eq!(other_copy["gear_id"], spare.to_string());
    assert_eq!(other_copy["source"], "declared");

    // "None of my pairs" is a choice no default overrides.
    let (status, body) = call(
        &resources,
        &athlete,
        Method::PUT,
        "/api/me/activities/intervals_icu/i1/gear",
        Some(json!({"gear_id": null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["gear_id"].is_null());
    assert_eq!(body["source"], "declared");
    for id in [road, spare] {
        let now = pair(&resources, &athlete, id).await;
        assert!(meters(&now).abs() < 1e-6, "{now}");
    }
}

#[tokio::test]
async fn the_warning_is_reached_at_the_pairs_own_threshold() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "gear-warn").await;
    let id = declare(
        &resources,
        &athlete,
        "Old faithful",
        490_000.0,
        500_000.0,
        10,
    )
    .await;
    cache(
        &resources,
        &athlete,
        "strava",
        &[run("w1", "strava", days_ago(3), 9_999.0)],
    )
    .await;
    let below = pair(&resources, &athlete, id).await;
    assert!((meters(&below) - 499_999.0).abs() < 1e-6, "{below}");
    assert_eq!(below["warn_reached"], false);

    cache(
        &resources,
        &athlete,
        "strava",
        &[run("w2", "strava", days_ago(1), 1.0)],
    )
    .await;
    let at = pair(&resources, &athlete, id).await;
    assert!((meters(&at) - 500_000.0).abs() < 1e-6, "{at}");
    assert_eq!(at["warn_reached"], true, "500 km on a 500 km threshold");

    // The athlete's own threshold decides.
    let (status, raised) = call(
        &resources,
        &athlete,
        Method::PATCH,
        &format!("/api/me/gear/{id}"),
        Some(json!({"warn_distance_meters": 800_000.0})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{raised}");
    assert_eq!(raised["warn_reached"], false);
    assert_eq!(
        raised["name"], "Old faithful",
        "an absent field keeps its value"
    );
    assert!((meters(&raised) - 500_000.0).abs() < 1e-6, "{raised}");
}

#[tokio::test]
async fn a_pair_with_history_is_retired_rather_than_deleted() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "gear-retire").await;
    let worn = declare(&resources, &athlete, "Worn", 0.0, 500_000.0, 10).await;
    cache(
        &resources,
        &athlete,
        "strava",
        &[run("r1", "strava", days_ago(5), 21_100.0)],
    )
    .await;
    assert_eq!(pair(&resources, &athlete, worn).await["activity_count"], 1);

    let (status, body) = call(
        &resources,
        &athlete,
        Method::DELETE,
        &format!("/api/me/gear/{worn}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "ResourceLocked");

    let (status, retired) = call(
        &resources,
        &athlete,
        Method::POST,
        &format!("/api/me/gear/{worn}/retire"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{retired}");
    assert!(retired["retired_at"].is_string());
    assert!((meters(&retired) - 21_100.0).abs() < 1e-6, "{retired}");
    assert_eq!(retired["is_default"], false);

    // A retired pair takes no new run and cannot be chosen.
    cache(
        &resources,
        &athlete,
        "strava",
        &[run("r2", "strava", days_ago(1), 5_000.0)],
    )
    .await;
    assert_eq!(pair(&resources, &athlete, worn).await["activity_count"], 1);
    let (status, body) = call(
        &resources,
        &athlete,
        Method::PUT,
        "/api/me/activities/strava/r2/gear",
        Some(json!({"gear_id": worn.to_string()})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, restored) = call(
        &resources,
        &athlete,
        Method::POST,
        &format!("/api/me/gear/{worn}/restore"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    assert!(restored["retired_at"].is_null());

    let (status, created) = call(
        &resources,
        &athlete,
        Method::POST,
        "/api/me/gear",
        Some(json!({"name": "Unworn", "sport": "trail_running"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let unworn = created["id"].as_str().unwrap().to_owned();
    let (status, _) = call(
        &resources,
        &athlete,
        Method::DELETE,
        &format!("/api/me/gear/{unworn}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(ledger(&resources, &athlete)
        .await
        .iter()
        .all(|pair| pair["id"] != unworn.as_str()));
}

#[tokio::test]
async fn deleting_an_uploaded_run_takes_it_off_its_pair_and_frees_the_pair() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "gear-upload-delete").await;
    let id = declare(&resources, &athlete, "Daily", 0.0, 500_000.0, 10).await;
    let workout = fit_run(days_ago(1));
    let upload_id = upload(&resources, &athlete, &workout).await;

    let worn = pair(&resources, &athlete, id).await;
    assert_eq!(worn["activity_count"], 1, "{worn}");
    assert!(
        (meters(&worn) - workout.distance_meters()).abs() < 1e-6,
        "{worn}"
    );
    let (status, body) = call(
        &resources,
        &athlete,
        Method::DELETE,
        &format!("/api/me/gear/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    delete_upload(&resources, &athlete, &upload_id).await;

    let freed = pair(&resources, &athlete, id).await;
    assert_eq!(freed["activity_count"], 0, "{freed}");
    assert!(meters(&freed).abs() < 1e-6, "{freed}");
    let (status, body) = call(
        &resources,
        &athlete,
        Method::DELETE,
        &format!("/api/me/gear/{id}"),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "nothing is attributed any more: {body}"
    );
}

#[tokio::test]
async fn a_choice_on_a_deleted_upload_stays_with_the_workouts_strava_copy() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "gear-upload-copy").await;
    let road = declare(&resources, &athlete, "Road", 0.0, 500_000.0, 10).await;
    let spare = declare(&resources, &athlete, "Spare", 0.0, 500_000.0, 12).await;
    let start = days_ago(1);
    let workout = fit_run(start);
    let upload_id = upload(&resources, &athlete, &workout).await;
    // Strava syncs the same run after it was uploaded: two copies, one workout.
    let strava = ActivityBuilder::new(
        "s9",
        "Morning Run",
        SportType::Run,
        start + Duration::seconds(20),
        600,
        "strava",
    )
    .distance_meters(workout.distance_meters())
    .start_latitude(45.5)
    .start_longitude(-73.6)
    .build();
    cache(&resources, &athlete, "strava", &[strava]).await;
    let (status, body) = call(
        &resources,
        &athlete,
        Method::PUT,
        &format!("/api/me/activities/upload/{upload_id}/gear"),
        Some(json!({"gear_id": spare.to_string()})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(pair(&resources, &athlete, spare).await["activity_count"], 1);
    assert_eq!(pair(&resources, &athlete, road).await["activity_count"], 0);

    delete_upload(&resources, &athlete, &upload_id).await;

    let (status, body) = call(
        &resources,
        &athlete,
        Method::GET,
        "/api/me/activities/strava/s9/gear",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["gear_id"], spare.to_string(), "{body}");
    assert_eq!(body["source"], "declared", "{body}");
    let spare_now = pair(&resources, &athlete, spare).await;
    assert_eq!(spare_now["activity_count"], 1, "{spare_now}");
    assert!(
        (meters(&spare_now) - workout.distance_meters()).abs() < 1e-6,
        "{spare_now}"
    );
    let road_now = pair(&resources, &athlete, road).await;
    assert_eq!(
        road_now["activity_count"], 0,
        "the remaining copy is not given a default on top: {road_now}"
    );
}

#[tokio::test]
async fn another_athlete_can_neither_read_nor_attribute_a_pair() {
    let resources = common::create_test_server_resources().await.unwrap();
    let owner = seed_athlete(&resources, "gear-owner").await;
    let other = seed_athlete(&resources, "gear-other").await;
    let id = declare(&resources, &owner, "Mine", 0.0, 500_000.0, 10).await;
    cache(
        &resources,
        &owner,
        "strava",
        &[run("o1", "strava", days_ago(2), 7_000.0)],
    )
    .await;
    cache(
        &resources,
        &other,
        "strava",
        &[run("x1", "strava", days_ago(2), 9_000.0)],
    )
    .await;

    assert!(ledger(&resources, &other).await.is_empty());
    let pair_uri = format!("/api/me/gear/{id}");
    for (method, uri, body) in [
        (
            Method::PATCH,
            pair_uri.clone(),
            Some(json!({"name": "Taken"})),
        ),
        (Method::POST, format!("{pair_uri}/retire"), None),
        (Method::DELETE, pair_uri.clone(), None),
        (
            Method::PUT,
            "/api/me/activities/strava/x1/gear".to_owned(),
            Some(json!({"gear_id": id.to_string()})),
        ),
        (
            Method::GET,
            "/api/me/activities/strava/o1/gear".to_owned(),
            None,
        ),
        (
            Method::PUT,
            "/api/me/activities/strava/o1/gear".to_owned(),
            Some(json!({"gear_id": null})),
        ),
    ] {
        let (status, answer) = call(&resources, &other, method.clone(), &uri, body).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri} → {answer}");
    }

    let mine = pair(&resources, &owner, id).await;
    assert_eq!(mine["name"], "Mine");
    assert!(mine["retired_at"].is_null());
    assert_eq!(
        mine["activity_count"], 1,
        "only the owner's own run: {mine}"
    );
    assert!((meters(&mine) - 7_000.0).abs() < 1e-6, "{mine}");
}

#[tokio::test]
async fn every_providers_recording_device_reaches_the_home_row() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "gear-device").await;
    let coros = ActivityBuilder::new("c1", "Run c1", SportType::Run, days_ago(1), 2_400, "strava")
        .source("strava")
        .device_name("COROS PACE 3")
        .build();
    let garmin = ActivityBuilder::new("g1", "Run g1", SportType::Run, days_ago(3), 2_400, "strava")
        .source("strava")
        .device_name("Garmin Forerunner 965")
        .build();
    cache(&resources, &athlete, "strava", &[coros, garmin]).await;

    let response = athlete_home_routes()
        .with_state(Arc::clone(&resources))
        .oneshot(
            Request::builder()
                .uri("/api/me/activities/recent")
                .header("authorization", format!("Bearer {}", athlete.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    let rows = body["activities"].as_array().unwrap();
    assert_eq!(rows[0]["attribution"], "COROS PACE 3", "{body}");
    assert_eq!(rows[1]["attribution"], "Garmin Forerunner 965", "{body}");
}
