// ABOUTME: POST /api/me/activities/upload — a completed workout's .fit becomes the athlete's own activity everywhere
// ABOUTME: Home, the calendar, the volume, the activity view and route, and the agent's get_activities all read it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Activity file upload suite (carnet#818).
//!
//! Every upload is a FIT file written by `pierre_test_support::fit` at a
//! start the test chooses — inside the cache's retention window, or at the
//! same minute as a provider's copy — and every read is the real route or
//! tool reading it back, so a handler that stored nothing, the wrong tenant's
//! rows or a fabricated figure fails on content.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
#![cfg(feature = "protocol-rest")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use chrono::{DateTime, Duration, Utc};
use pierre_core::constants::oauth_providers::UPLOAD;
use pierre_core::models::{ActivityBuilder, SportType, Tenant, TenantId, User};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use pierre_mcp_server::services::activity_upload::MAX_UPLOAD_BYTES;
use pierre_test_support::fit::{course_file, FitWorkout};
use pierre_tool_runtime::protocol::{UniversalRequest, UniversalToolExecutor};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

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
async fn second_tenant(resources: &Arc<ServerContext>, athlete: &Athlete) -> String {
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
    resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &athlete.user,
            &resources.auth.jwks_manager,
            Some(tenant.to_string()),
        )
        .unwrap()
}

async fn send(resources: &Arc<ServerContext>, request: Request<Body>) -> (StatusCode, Value) {
    let response = athlete_home_routes()
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

async fn upload(resources: &Arc<ServerContext>, token: &str, file: Vec<u8>) -> (StatusCode, Value) {
    send(
        resources,
        Request::builder()
            .method("POST")
            .uri("/api/me/activities/upload")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/octet-stream")
            .body(Body::from(file))
            .unwrap(),
    )
    .await
}

async fn get(resources: &Arc<ServerContext>, token: &str, uri: &str) -> (StatusCode, Value) {
    send(
        resources,
        Request::builder()
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await
}

/// Yesterday at 07:00 UTC: inside every Home read's window.
fn yesterday_morning() -> DateTime<Utc> {
    (Utc::now() - Duration::days(1))
        .date_naive()
        .and_hms_opt(7, 0, 0)
        .unwrap()
        .and_utc()
}

/// A ten-minute ride, uploaded, and the Home row it became.
async fn uploaded_ride(resources: &Arc<ServerContext>, athlete: &Athlete) -> Value {
    let (status, body) = upload(
        resources,
        &athlete.token,
        FitWorkout::ride(yesterday_morning(), 600).encode(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["activities"][0].clone()
}

fn ids(rows: &Value) -> Vec<String> {
    rows.as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn an_uploaded_ride_becomes_an_activity_holding_only_what_the_file_recorded() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "upload-row").await;
    let workout = FitWorkout::ride(yesterday_morning(), 600);

    let (status, body) = upload(&resources, &athlete.token, workout.encode()).await;

    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["already_held"], json!([]));
    let row = &body["activities"][0];
    assert_eq!(row["provider"], json!(UPLOAD));
    assert!(row["id"].as_str().unwrap().ends_with("-0"), "{row}");
    assert_eq!(row["name"], json!(""), "a FIT session carries no title");
    assert_eq!(row["start_date"], json!(yesterday_morning()));
    assert_eq!(row["duration_seconds"], json!(600));
    assert_eq!(row["distance_meters"], json!(workout.distance_meters()));
    assert_eq!(
        row["elevation_gain_meters"],
        Value::Null,
        "the file records none"
    );
    assert_eq!(row["has_gps"], json!(true));
}

#[tokio::test]
async fn home_the_calendar_and_the_volume_all_read_the_upload() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "upload-home").await;
    let row = uploaded_ride(&resources, &athlete).await;
    let id = row["id"].as_str().unwrap().to_owned();

    let (_, recent) = get(&resources, &athlete.token, "/api/me/activities/recent").await;
    assert_eq!(ids(&recent["activities"]), vec![id.clone()]);
    assert_eq!(
        recent["as_of"],
        Value::Null,
        "an upload is not a provider sync"
    );

    let day = yesterday_morning().date_naive();
    let (status, calendar) = get(
        &resources,
        &athlete.token,
        &format!("/api/me/calendar?from={day}&to={day}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{calendar}");
    let on_day = calendar["activities"].as_array().unwrap();
    assert_eq!(on_day.len(), 1, "{calendar}");
    assert_eq!(on_day[0]["activity"]["id"], json!(id));

    let (status, volume) = get(&resources, &athlete.token, "/api/me/training-volume").await;
    assert_eq!(status, StatusCode::OK, "{volume}");
    let sessions: u64 = volume["weeks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|week| week["sports"].as_array().unwrap().iter())
        .map(|sport| sport["activities"].as_u64().unwrap())
        .sum();
    assert_eq!(sessions, 1, "{volume}");
}

#[tokio::test]
async fn the_activity_view_reads_the_laps_and_the_route_from_the_file() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "upload-view").await;
    let row = uploaded_ride(&resources, &athlete).await;
    let id = row["id"].as_str().unwrap();

    let (status, view) = get(
        &resources,
        &athlete.token,
        &format!("/api/me/activities/upload/{id}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["laps"].as_array().unwrap().len(), 1, "{view}");
    assert!(view["average_heart_rate"].as_u64().is_some(), "{view}");
    assert_eq!(view["average_power"], Value::Null, "no power in the file");

    let (status, route) = get(
        &resources,
        &athlete.token,
        &format!("/api/me/activities/upload/{id}/route"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{route}");
    // The ride heads due north for 4.8 km: trimmed at both ends and
    // simplified, the line runs from just past the start to just short of
    // the end.
    let coordinates = route["route"]["coordinates"].as_array().unwrap();
    assert!(coordinates.len() >= 2, "{route}");
    let first = coordinates[0][0].as_f64().unwrap();
    let last = coordinates[coordinates.len() - 1][0].as_f64().unwrap();
    assert!(first > 45.5 && last > first + 0.03, "{route}");
    assert_eq!(route["route"]["source_tool"], json!(UPLOAD), "{route}");
}

#[tokio::test]
async fn an_upload_without_a_bearer_is_refused() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (status, _) = send(
        &resources,
        Request::builder()
            .method("POST")
            .uri("/api/me/activities/upload")
            .body(Body::from(
                FitWorkout::ride(yesterday_morning(), 60).encode(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_body_past_the_limit_is_refused_before_it_is_read() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "upload-large").await;
    let (status, _) = upload(&resources, &athlete.token, vec![0; MAX_UPLOAD_BYTES + 1]).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn a_file_that_is_not_a_completed_activity_is_refused_and_nothing_is_kept() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "upload-bad").await;
    let mut corrupted = FitWorkout::ride(yesterday_morning(), 60).encode();
    let middle = corrupted.len() / 2;
    corrupted[middle] ^= 0xFF;

    for (label, file) in [
        ("empty", Vec::new()),
        ("not a FIT file", b"<gpx></gpx>".to_vec()),
        ("a course", course_file(yesterday_morning())),
        ("a broken checksum", corrupted),
    ] {
        let (status, body) = upload(&resources, &athlete.token, file).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{label}: {body}");
    }
    let held = resources
        .common
        .repos
        .uploaded_activity_files
        .has_uploaded_files(&athlete.tenant, athlete.user_id)
        .await
        .unwrap();
    assert!(!held, "a refused upload keeps nothing");
}

#[tokio::test]
async fn the_same_file_uploaded_twice_is_one_activity() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "upload-twice").await;
    let file = FitWorkout::ride(yesterday_morning(), 600).encode();

    let (first, _) = upload(&resources, &athlete.token, file.clone()).await;
    let (second, body) = upload(&resources, &athlete.token, file).await;

    assert_eq!(first, StatusCode::CREATED);
    assert_eq!(second, StatusCode::CONFLICT, "{body}");
    let (_, recent) = get(&resources, &athlete.token, "/api/me/activities/recent").await;
    assert_eq!(recent["activities"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_workout_a_provider_already_synced_is_not_uploaded_again() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "upload-dup").await;
    let workout = FitWorkout::ride(yesterday_morning(), 600);
    let strava = ActivityBuilder::new(
        "998877",
        "Morning Ride",
        SportType::Ride,
        yesterday_morning() + Duration::seconds(20),
        600,
        "strava",
    )
    .distance_meters(workout.distance_meters())
    .start_latitude(45.5)
    .start_longitude(-73.6)
    .build();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(athlete.user_id, &athlete.tenant, "strava", &[strava])
        .await
        .unwrap();

    let (status, body) = upload(&resources, &athlete.token, workout.encode()).await;

    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], json!("ResourceAlreadyExists"), "{body}");
    let (_, recent) = get(&resources, &athlete.token, "/api/me/activities/recent").await;
    assert_eq!(ids(&recent["activities"]), vec!["998877".to_owned()]);
}

#[tokio::test]
async fn an_upload_is_its_uploaders_alone_in_the_tenant_it_was_made_in() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "upload-owner").await;
    let row = uploaded_ride(&resources, &athlete).await;
    let id = row["id"].as_str().unwrap();

    let other_tenant = second_tenant(&resources, &athlete).await;
    let (_, recent) = get(&resources, &other_tenant, "/api/me/activities/recent").await;
    assert_eq!(recent["activities"], json!([]));

    let stranger = seed_athlete(&resources, "upload-stranger").await;
    for uri in [
        format!("/api/me/activities/upload/{id}"),
        format!("/api/me/activities/upload/{id}/route"),
    ] {
        let (status, body) = get(&resources, &stranger.token, &uri).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}: {body}");
    }
}

#[tokio::test]
async fn the_retention_prune_keeps_an_uploaded_workout() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "upload-prune").await;
    let row = uploaded_ride(&resources, &athlete).await;

    let pruned = resources
        .common
        .repos
        .activity_cache
        .prune_activities_before(athlete.user_id, &athlete.tenant, Utc::now())
        .await
        .unwrap();

    assert_eq!(pruned, 0, "an upload is the record, not a copy to refetch");
    let kept = resources
        .common
        .repos
        .activity_cache
        .get_cached_activity(
            athlete.user_id,
            &athlete.tenant,
            UPLOAD,
            row["id"].as_str().unwrap(),
        )
        .await
        .unwrap();
    assert!(kept.is_some());
}

#[tokio::test]
async fn an_upload_is_not_read_as_a_provider_sync() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "upload-sync").await;
    uploaded_ride(&resources, &athlete).await;

    let synced = resources
        .common
        .repos
        .activity_cache
        .latest_activity_sync_any(athlete.user_id, &athlete.tenant)
        .await
        .unwrap();

    assert_eq!(
        synced, None,
        "an upload's synced_at is when the athlete uploaded, not a provider sync"
    );
}

/// `get_activities` as the agent calls it, in the athlete's tenant.
async fn agent_activities(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    parameters: Value,
) -> Value {
    let response = UniversalToolExecutor::new(resources.clone())
        .with_scopes(OAuthScope::self_grant())
        .with_transport(Transport::WebApp)
        .execute_tool(UniversalRequest {
            tool_name: "get_activities".to_owned(),
            parameters,
            user_id: athlete.user_id.to_string(),
            protocol: "mcp".to_owned(),
            tenant_id: Some(athlete.tenant.to_string()),
        })
        .await
        .expect("the tool dispatches");
    assert!(response.success, "get_activities ran: {:?}", response.error);
    response.result.expect("the tool returned a payload")
}

#[tokio::test]
async fn the_agent_reads_an_upload_with_no_provider_connected() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "upload-agent").await;
    let row = uploaded_ride(&resources, &athlete).await;

    let payload = agent_activities(&resources, &athlete, json!({ "limit": 5 })).await;

    assert_eq!(
        ids(&payload["activities"]),
        vec![row["id"].as_str().unwrap().to_owned()],
        "{payload}"
    );
    let named = agent_activities(&resources, &athlete, json!({ "provider": UPLOAD })).await;
    assert_eq!(named["activities"].as_array().unwrap().len(), 1, "{named}");
}

#[tokio::test]
async fn an_athlete_with_neither_a_provider_nor_an_upload_is_still_told_to_connect() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "upload-none").await;
    let response = UniversalToolExecutor::new(resources.clone())
        .with_scopes(OAuthScope::self_grant())
        .with_transport(Transport::WebApp)
        .execute_tool(UniversalRequest {
            tool_name: "get_activities".to_owned(),
            parameters: json!({}),
            user_id: athlete.user_id.to_string(),
            protocol: "mcp".to_owned(),
            tenant_id: Some(athlete.tenant.to_string()),
        })
        .await
        .expect("the tool dispatches");
    assert!(
        response
            .result
            .as_ref()
            .is_some_and(|result| { result.to_string().to_lowercase().contains("connect") })
            || response
                .error
                .as_ref()
                .is_some_and(|error| error.to_lowercase().contains("connect")),
        "{response:?}"
    );
}

async fn delete(resources: &Arc<ServerContext>, token: &str, id: &str) -> (StatusCode, Value) {
    delete_of(resources, token, UPLOAD, id).await
}

async fn delete_of(
    resources: &Arc<ServerContext>,
    token: &str,
    provider: &str,
    id: &str,
) -> (StatusCode, Value) {
    send(
        resources,
        Request::builder()
            .method("DELETE")
            .uri(format!("/api/me/activities/{provider}/{id}"))
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await
}

#[tokio::test]
async fn deleting_an_upload_removes_it_from_every_read_and_drops_the_file() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "upload-delete").await;
    let row = uploaded_ride(&resources, &athlete).await;
    let id = row["id"].as_str().unwrap();

    let (status, body) = delete(&resources, &athlete.token, id).await;

    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let (_, recent) = get(&resources, &athlete.token, "/api/me/activities/recent").await;
    assert_eq!(recent["activities"], json!([]));
    let day = yesterday_morning().date_naive();
    let (_, calendar) = get(
        &resources,
        &athlete.token,
        &format!("/api/me/calendar?from={day}&to={day}"),
    )
    .await;
    assert_eq!(calendar["activities"], json!([]), "{calendar}");
    let (status, _) = get(
        &resources,
        &athlete.token,
        &format!("/api/me/activities/upload/{id}"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let held = resources
        .common
        .repos
        .uploaded_activity_files
        .has_uploaded_files(&athlete.tenant, athlete.user_id)
        .await
        .unwrap();
    assert!(!held, "the file goes with its last activity");

    let (again, _) = delete(&resources, &athlete.token, id).await;
    assert_eq!(again, StatusCode::NOT_FOUND, "a deleted upload is gone");
    let (status, body) = upload(
        &resources,
        &athlete.token,
        FitWorkout::ride(yesterday_morning(), 600).encode(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the same file can be uploaded again: {body}"
    );
}

#[tokio::test]
async fn only_the_uploader_can_delete_an_upload_and_only_an_upload() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "upload-delete-owner").await;
    let row = uploaded_ride(&resources, &athlete).await;
    let id = row["id"].as_str().unwrap();

    let stranger = seed_athlete(&resources, "upload-delete-stranger").await;
    let (status, _) = delete(&resources, &stranger.token, id).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "another athlete's upload");
    let other_tenant = second_tenant(&resources, &athlete).await;
    let (status, _) = delete(&resources, &other_tenant, id).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the uploader in another tenant"
    );

    let strava = ActivityBuilder::new(
        "445566",
        "Evening Run",
        SportType::Run,
        yesterday_morning() + Duration::hours(10),
        1_800,
        "strava",
    )
    .build();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(athlete.user_id, &athlete.tenant, "strava", &[strava])
        .await
        .unwrap();
    let (status, _) = delete_of(&resources, &athlete.token, "strava", "445566").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a provider's activity");
    let (status, _) = delete_of(&resources, &athlete.token, "strava", id).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "an upload's id under a provider"
    );

    let (_, recent) = get(&resources, &athlete.token, "/api/me/activities/recent").await;
    assert_eq!(
        recent["activities"].as_array().unwrap().len(),
        2,
        "nothing was deleted: {recent}"
    );
    let (status, _) = delete(&resources, &athlete.token, id).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "the uploader still can");
}
