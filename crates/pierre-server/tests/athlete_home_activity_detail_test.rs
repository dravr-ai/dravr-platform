// ABOUTME: GET /api/me/activities/{provider}/{activity_id} — one workout's view: its Home row, figures, splits and laps
// ABOUTME: Cache-served, merged as Home merges it, null where unrecorded, 404 outside the caller's rows; names only the caller's own linked thread

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Athlete Home activity-detail suite.
//!
//! Every test seeds real rows through the activity cache's own writer and
//! reads the route's JSON back, so a handler that answered fixed figures,
//! dropped the splits or served another athlete's row fails on content.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
#![cfg(feature = "provider-strava")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::slice;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use chrono::{DateTime, Duration, Utc};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{
    Activity, ActivityBuilder, Lap, Split, SportType, Tenant, TenantId, User,
};
use pierre_database::repositories::ActivityDetail;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use pierre_mcp_server::services::activity_detail::read_activity_detail;
use pierre_mcp_server::services::activity_route::CachedActivityRef;
use pierre_mcp_server::tools::runtime_adapter::into_runtime;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

/// Every key the detail body carries, null or not.
const DETAIL_KEYS: [&str; 10] = [
    "activity",
    "average_heart_rate",
    "max_heart_rate",
    "average_speed_mps",
    "max_speed_mps",
    "average_power",
    "calories",
    "splits",
    "laps",
    "conversation_id",
];

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

async fn put_link(
    resources: &Arc<ServerContext>,
    token: &str,
    uri: &str,
    conversation_id: Option<&str>,
) -> (StatusCode, Value) {
    let response = athlete_home_routes()
        .with_state(Arc::clone(resources))
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "conversation_id": conversation_id }).to_string(),
                ))
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

/// A conversation `athlete` owns in their own tenant, as the chat surface opens one.
async fn open_conversation(resources: &Arc<ServerContext>, athlete: &Athlete) -> String {
    resources
        .common
        .repos
        .chat
        .create_conversation(
            &athlete.user_id.to_string(),
            athlete.tenant,
            "Tempo 10k",
            "gemini-2.0-flash",
            None,
            None,
        )
        .await
        .unwrap()
        .id
}

async fn detail(resources: &Arc<ServerContext>, token: &str, provider: &str, id: &str) -> Value {
    let (status, body) = get_json(
        resources,
        token,
        &format!("/api/me/activities/{provider}/{id}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
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

/// A fixed hour `days` days back.
fn days_ago(days: i64) -> DateTime<Utc> {
    Utc::now()
        .date_naive()
        .and_hms_opt(6, 0, 0)
        .unwrap()
        .and_utc()
        - Duration::days(days)
}

/// A 10 km tempo run as a detailed read caches it: heart rate, speed, power,
/// cadence, energy, two kilometre splits and one lap.
fn detailed_run(started: DateTime<Utc>) -> Activity {
    ActivityBuilder::new(
        "tempo-10k",
        "Tempo 10k",
        SportType::Run,
        started,
        2_700,
        oauth_providers::STRAVA,
    )
    .distance_meters(10_000.0)
    .elevation_gain(64.0)
    .average_heart_rate(158)
    .max_heart_rate(176)
    .average_speed(3.9)
    .max_speed(5.2)
    .average_power(287)
    .average_cadence(172)
    .calories(712)
    .splits(vec![
        Split {
            index: 1,
            distance_meters: 1_000.0,
            elapsed_time_seconds: 262,
            moving_time_seconds: Some(260),
            elevation_difference_meters: Some(4.2),
            average_speed_mps: Some(3.85),
            average_heart_rate: Some(151),
            pace_zone: Some(2),
        },
        Split {
            index: 2,
            distance_meters: 1_000.0,
            elapsed_time_seconds: 251,
            moving_time_seconds: None,
            elevation_difference_meters: Some(-2.5),
            average_speed_mps: Some(3.98),
            average_heart_rate: None,
            pace_zone: None,
        },
    ])
    .laps(vec![Lap {
        id: Some("lap-1".to_owned()),
        index: 1,
        distance_meters: 10_000.0,
        elapsed_time_seconds: 2_700,
        moving_time_seconds: Some(2_560),
        elevation_gain_meters: Some(64.0),
        average_speed_mps: Some(3.9),
        max_speed_mps: Some(5.2),
        average_heart_rate: Some(158),
        max_heart_rate: Some(176),
        average_cadence: Some(172),
        average_power: Some(287),
    }])
    .build()
}

#[tokio::test]
async fn the_view_carries_every_figure_split_and_lap_the_cache_holds() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "detail-figures").await;
    let started = days_ago(1);
    cache(
        &resources,
        &athlete,
        oauth_providers::STRAVA,
        &[detailed_run(started)],
    )
    .await;

    let body = detail(&resources, &athlete.token, "strava", "tempo-10k").await;

    let keys = body.as_object().unwrap();
    assert_eq!(keys.len(), DETAIL_KEYS.len(), "{body}");
    for key in DETAIL_KEYS {
        assert!(keys.contains_key(key), "missing {key}: {body}");
    }
    let row = &body["activity"];
    assert_eq!(row["id"], "tempo-10k");
    assert_eq!(row["provider"], "strava");
    assert_eq!(row["name"], "Tempo 10k");
    assert_eq!(row["sport_type"], "run");
    assert_eq!(row["duration_seconds"], 2_700);
    assert_eq!(row["distance_meters"], 10_000.0);
    assert_eq!(row["elevation_gain_meters"], 64.0);
    assert_eq!(row["has_gps"], true);
    assert_eq!(
        DateTime::parse_from_rfc3339(row["start_date"].as_str().unwrap()).unwrap(),
        started
    );
    assert_eq!(body["average_heart_rate"], 158);
    assert_eq!(body["max_heart_rate"], 176);
    assert_eq!(body["average_speed_mps"], 3.9);
    assert_eq!(body["max_speed_mps"], 5.2);
    assert_eq!(body["average_power"], 287);
    assert_eq!(body["calories"], 712);
    assert_eq!(
        body["splits"],
        json!([
            {
                "index": 1,
                "distance_meters": 1_000.0,
                "elapsed_time_seconds": 262,
                "moving_time_seconds": 260,
                "elevation_difference_meters": 4.2,
                "average_speed_mps": 3.85,
                "average_heart_rate": 151,
            },
            {
                "index": 2,
                "distance_meters": 1_000.0,
                "elapsed_time_seconds": 251,
                "moving_time_seconds": null,
                "elevation_difference_meters": -2.5,
                "average_speed_mps": 3.98,
                "average_heart_rate": null,
            },
        ]),
        "every split key is present, null where the provider sent nothing"
    );
    assert_eq!(
        body["laps"],
        json!([{
            "index": 1,
            "distance_meters": 10_000.0,
            "elapsed_time_seconds": 2_700,
            "moving_time_seconds": 2_560,
            "elevation_gain_meters": 64.0,
            "average_speed_mps": 3.9,
            "average_heart_rate": 158,
            "max_heart_rate": 176,
            "average_power": 287,
        }])
    );
}

#[tokio::test]
async fn a_figure_the_cache_does_not_hold_is_null_and_no_split_is_made_up() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "detail-bare").await;
    let bare = ActivityBuilder::new(
        "yoga-1",
        "Evening yoga",
        SportType::Yoga,
        days_ago(3),
        1_800,
        oauth_providers::STRAVA,
    )
    .build();
    cache(&resources, &athlete, oauth_providers::STRAVA, &[bare]).await;

    let body = detail(&resources, &athlete.token, "strava", "yoga-1").await;

    for key in [
        "average_heart_rate",
        "max_heart_rate",
        "average_speed_mps",
        "max_speed_mps",
        "average_power",
        "calories",
    ] {
        assert!(body[key].is_null(), "{key} must be null: {body}");
    }
    assert_eq!(body["splits"], json!([]));
    assert_eq!(body["laps"], json!([]));
    assert!(body["activity"]["distance_meters"].is_null(), "{body}");
    assert_eq!(body["activity"]["name"], "Evening yoga");
}

/// The 2026-09-28 run as jf@dravr.ai's Strava mirror caches it: a manual
/// entry and the watch's recording of the same run.
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
async fn a_merged_workout_reads_as_its_home_row_from_either_copy() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "detail-merged").await;
    cache(
        &resources,
        &athlete,
        oauth_providers::SCIOTTE,
        &manual_and_recorded_run(days_ago(2)),
    )
    .await;
    let (status, recent) = get_json(&resources, &athlete.token, "/api/me/activities/recent").await;
    assert_eq!(status, StatusCode::OK, "{recent}");
    let home_row = recent["activities"][0].clone();

    for id in ["morning-trail-run", "bug-de-fougeres"] {
        let body = detail(&resources, &athlete.token, "strava", id).await;
        assert_eq!(
            body["activity"], home_row,
            "the view of {id} is the Home row, merged the same way"
        );
        assert_eq!(body["activity"]["id"], "morning-trail-run", "the GPS copy");
        assert_eq!(body["activity"]["name"], "Bug de Fougères");
        assert_eq!(
            body["average_heart_rate"], 152,
            "the heart rate the recording carried, merged in"
        );
    }
}

#[tokio::test]
async fn another_users_or_tenants_activity_and_an_unknown_one_are_not_found() {
    let resources = common::create_test_server_resources().await.unwrap();
    let owner = seed_athlete(&resources, "detail-owner").await;
    let stranger = seed_athlete(&resources, "detail-stranger").await;
    cache(
        &resources,
        &owner,
        oauth_providers::STRAVA,
        &[detailed_run(days_ago(1))],
    )
    .await;

    let (status, body) = get_json(
        &resources,
        &stranger.token,
        "/api/me/activities/strava/tempo-10k",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // The owner acting in a tenant that holds none of their rows.
    let other_tenant = TenantId::generate();
    resources
        .common
        .repos
        .tenants
        .create(&Tenant {
            id: other_tenant,
            name: "second tenant".to_owned(),
            slug: format!("second-{other_tenant}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: owner.user_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    let token = resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &owner.user,
            &resources.auth.jwks_manager,
            Some(other_tenant.to_string()),
        )
        .unwrap();
    let (status, body) = get_json(&resources, &token, "/api/me/activities/strava/tempo-10k").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    let (status, body) = get_json(
        &resources,
        &owner.token,
        "/api/me/activities/strava/never-cached",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // The owner still reads their own.
    let body = detail(&resources, &owner.token, "strava", "tempo-10k").await;
    assert_eq!(body["activity"]["id"], "tempo-10k");
}

// A detail read takes the athlete's provider turn, and one that waited behind
// a read of the same activity finds that read's splits and laps in the cache
// and asks the provider nothing. The athlete here holds no Strava connection,
// so a read that went to the provider would fail and answer `false`.
#[tokio::test]
async fn a_detail_read_behind_one_that_stored_the_detail_asks_no_provider() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "detail-read-queued").await;
    let started = days_ago(1);
    let listed = ActivityBuilder::new(
        "tempo-10k",
        "Tempo 10k",
        SportType::Run,
        started,
        2_700,
        oauth_providers::STRAVA,
    )
    .distance_meters(10_000.0)
    .build();
    cache(
        &resources,
        &athlete,
        oauth_providers::STRAVA,
        slice::from_ref(&listed),
    )
    .await;
    // The read that held the turn first stored what it found.
    let stored_first = resources
        .common
        .repos
        .activity_cache
        .store_activity_detail(
            athlete.user_id,
            &athlete.tenant,
            oauth_providers::STRAVA,
            "tempo-10k",
            &ActivityDetail::from_activity(&detailed_run(started)),
            None,
        )
        .await
        .unwrap();
    assert!(stored_first);

    let runtime = into_runtime(&resources);
    let answered = read_activity_detail(
        &runtime,
        &resources.common.turns,
        athlete.tenant,
        athlete.user_id,
        CachedActivityRef {
            provider: oauth_providers::STRAVA,
            activity: &listed,
        },
    )
    .await;
    assert!(
        answered,
        "the detail the first read stored answers the second, without a provider read"
    );
    let body = detail(&resources, &athlete.token, "strava", "tempo-10k").await;
    assert_eq!(body["splits"].as_array().map(Vec::len), Some(2), "{body}");
}

// The Home row names the copy whose route it draws, and that copy changes
// when a copy's route read settles. A thread linked while the row named one
// copy of the workout is still the workout's thread when the view is opened
// on another.
#[tokio::test]
async fn a_thread_linked_on_one_copy_is_named_by_the_view_of_every_copy() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "detail-thread-copies").await;
    cache(
        &resources,
        &athlete,
        oauth_providers::SCIOTTE,
        &manual_and_recorded_run(days_ago(2)),
    )
    .await;
    let thread = open_conversation(&resources, &athlete).await;
    let (status, body) = put_link(
        &resources,
        &athlete.token,
        "/api/me/activities/strava/bug-de-fougeres/conversation",
        Some(&thread),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    for id in ["bug-de-fougeres", "morning-trail-run"] {
        let body = detail(&resources, &athlete.token, "strava", id).await;
        assert_eq!(
            body["conversation_id"],
            thread.as_str(),
            "the view of {id} resumes the workout's thread: {body}"
        );
    }
}

#[tokio::test]
async fn the_thread_the_view_links_is_named_by_every_later_read_until_it_is_forgotten() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "detail-thread").await;
    cache(
        &resources,
        &athlete,
        oauth_providers::STRAVA,
        &[detailed_run(days_ago(1))],
    )
    .await;
    let uri = "/api/me/activities/strava/tempo-10k/conversation";

    // Before the view's first question there is no thread.
    let body = detail(&resources, &athlete.token, "strava", "tempo-10k").await;
    assert_eq!(body["conversation_id"], Value::Null, "{body}");

    let thread = open_conversation(&resources, &athlete).await;
    let (status, body) = put_link(&resources, &athlete.token, uri, Some(&thread)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["conversation_id"], thread.as_str());

    // A later read — any device, nothing cached client-side — names it.
    let body = detail(&resources, &athlete.token, "strava", "tempo-10k").await;
    assert_eq!(body["conversation_id"], thread.as_str(), "{body}");

    // A `/reset` moves the view to a fresh thread: the link follows it.
    let fresh = open_conversation(&resources, &athlete).await;
    let (status, _) = put_link(&resources, &athlete.token, uri, Some(&fresh)).await;
    assert_eq!(status, StatusCode::OK);
    let body = detail(&resources, &athlete.token, "strava", "tempo-10k").await;
    assert_eq!(body["conversation_id"], fresh.as_str(), "{body}");

    // Forgotten on `null`; the conversation itself stays.
    let (status, body) = put_link(&resources, &athlete.token, uri, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let body = detail(&resources, &athlete.token, "strava", "tempo-10k").await;
    assert_eq!(body["conversation_id"], Value::Null, "{body}");
    assert!(resources
        .common
        .repos
        .chat
        .get_conversation(&fresh, &athlete.user_id.to_string(), athlete.tenant)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn only_the_callers_own_conversation_links_only_to_the_callers_own_activity() {
    let resources = common::create_test_server_resources().await.unwrap();
    let owner = seed_athlete(&resources, "detail-thread-owner").await;
    let stranger = seed_athlete(&resources, "detail-thread-stranger").await;
    for athlete in [&owner, &stranger] {
        cache(
            &resources,
            athlete,
            oauth_providers::STRAVA,
            &[detailed_run(days_ago(1))],
        )
        .await;
    }
    let uri = "/api/me/activities/strava/tempo-10k/conversation";
    let owners_thread = open_conversation(&resources, &owner).await;
    let strangers_thread = open_conversation(&resources, &stranger).await;

    // Someone else's conversation cannot be linked to one's own activity.
    let (status, body) = put_link(&resources, &owner.token, uri, Some(&strangers_thread)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    // Nor a conversation that does not exist.
    let (status, body) = put_link(&resources, &owner.token, uri, Some("no-such-thread")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    // Nor anything to an activity the caller does not hold.
    let (status, body) = put_link(
        &resources,
        &owner.token,
        "/api/me/activities/strava/never-cached/conversation",
        Some(&owners_thread),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let body = detail(&resources, &owner.token, "strava", "tempo-10k").await;
    assert_eq!(body["conversation_id"], Value::Null, "{body}");

    // The owner's link is the owner's alone: the stranger's copy of the same
    // activity id reads no thread.
    let (status, _) = put_link(&resources, &owner.token, uri, Some(&owners_thread)).await;
    assert_eq!(status, StatusCode::OK);
    let body = detail(&resources, &stranger.token, "strava", "tempo-10k").await;
    assert_eq!(body["conversation_id"], Value::Null, "{body}");
    let body = detail(&resources, &owner.token, "strava", "tempo-10k").await;
    assert_eq!(body["conversation_id"], owners_thread.as_str(), "{body}");
}

#[tokio::test]
async fn a_deleted_conversation_takes_its_link_with_it() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "detail-thread-deleted").await;
    cache(
        &resources,
        &athlete,
        oauth_providers::STRAVA,
        &[detailed_run(days_ago(1))],
    )
    .await;
    let thread = open_conversation(&resources, &athlete).await;
    let (status, _) = put_link(
        &resources,
        &athlete.token,
        "/api/me/activities/strava/tempo-10k/conversation",
        Some(&thread),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert!(resources
        .common
        .repos
        .chat
        .delete_conversation(&thread, &athlete.user_id.to_string(), athlete.tenant)
        .await
        .unwrap());
    let body = detail(&resources, &athlete.token, "strava", "tempo-10k").await;
    assert_eq!(body["conversation_id"], Value::Null, "{body}");
    assert_eq!(
        resources
            .common
            .repos
            .activity_conversations
            .get_activity_conversation(
                &athlete.tenant,
                athlete.user_id,
                oauth_providers::STRAVA,
                "tempo-10k"
            )
            .await
            .unwrap(),
        None
    );
}
