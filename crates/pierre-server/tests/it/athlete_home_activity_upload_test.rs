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

use crate::common;

use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use bytes::Bytes;
use chrono::{DateTime, Duration, Utc};
use dravr_barrage::blob::memory::MemoryBlobStore;
use dravr_barrage::blob::{
    BlobKey, BlobMeta, BlobPrefix, BlobStat, BlobStore, DeletePage, PageToken, UploadTarget,
};
use dravr_barrage::digest::Sha256;
use dravr_barrage::parse::fit::FitFileError;
use dravr_barrage::{BarrageError, Scope};
use futures_util::stream::BoxStream;
use pierre_core::constants::oauth_providers::UPLOAD;
use pierre_core::models::{ActivityBuilder, SportType, Tenant, TenantId, User};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use pierre_mcp_server::services::activity_upload::MAX_UPLOAD_BYTES;
use pierre_providers::originals::OriginalsStore;
use pierre_providers::upload_provider::file_sha256;
use pierre_test_support::fit::{bloated_file, course_file, FitWorkout};
use pierre_tool_runtime::protocol::{
    is_no_provider_refusal, UniversalRequest, UniversalToolExecutor,
};
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
async fn the_activity_view_reads_the_uploads_laps_and_route() {
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
        let (status, body) = upload(&resources, &athlete.token, file.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{label}: {body}");
        assert!(
            !kept_file(&resources, &athlete, &file).await,
            "{label}: a refused upload keeps nothing"
        );
    }
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
        .prune_activities_before(athlete.user_id, &athlete.tenant, "upload", Utc::now())
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
    assert_told_to_connect(&resources, &athlete).await;
}

/// A reading the athlete typed in (a weight, a resting heart rate) is not a
/// data source: it is filed under the `athlete` key, and the provider-backed
/// tools still ask them to connect one.
#[tokio::test]
async fn a_reading_typed_by_hand_is_not_a_data_source() {
    use pierre_services::effective_physiology::{record_manual_measurements, ManualMeasurements};

    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "typed-reading").await;
    record_manual_measurements(
        &resources.common.repos,
        athlete.tenant,
        athlete.user_id,
        ManualMeasurements {
            resting_hr: None,
            weight_kg: Some(72.0),
        },
    )
    .await
    .unwrap();
    assert_told_to_connect(&resources, &athlete).await;
}

/// `get_activities` answers `athlete` with the prompt to connect a provider.
async fn assert_told_to_connect(resources: &Arc<ServerContext>, athlete: &Athlete) {
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
        is_no_provider_refusal(&response),
        "bug: the onboarding refusal did not fire: {response:?}"
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
    let (sha, _) = id.rsplit_once('-').unwrap();
    let kept = resources
        .common
        .originals
        .get_upload(&athlete.tenant, athlete.user_id, sha)
        .await
        .unwrap();
    assert!(kept.is_none(), "the file goes with its last activity");

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

/// A file's other session still cached keeps the file: the delete reads the
/// session ids the cache holds rather than decoding the file to count them,
/// and an id of another file is never one of them.
#[tokio::test]
async fn the_file_stays_while_another_of_its_sessions_is_cached() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "upload-delete-multi").await;
    let row = uploaded_ride(&resources, &athlete).await;
    let id = row["id"].as_str().unwrap();
    let (sha, _) = id.rsplit_once('-').unwrap();
    let second_id = format!("{sha}-1");
    let second_session = ActivityBuilder::new(
        second_id.as_str(),
        "",
        SportType::Run,
        yesterday_morning() + Duration::hours(1),
        600,
        UPLOAD,
    )
    .build();
    let other_file = FitWorkout::ride(yesterday_morning() - Duration::days(1), 300).encode();
    let (status, body) = upload(&resources, &athlete.token, other_file).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let repos = &resources.common.repos;
    repos
        .activity_cache
        .upsert_activities(athlete.user_id, &athlete.tenant, UPLOAD, &[second_session])
        .await
        .unwrap();
    let file_held = || async {
        resources
            .common
            .originals
            .get_upload(&athlete.tenant, athlete.user_id, sha)
            .await
            .unwrap()
            .is_some()
    };

    let (status, body) = delete(&resources, &athlete.token, id).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert!(file_held().await, "its second session still reads the file");

    let (status, body) = delete(&resources, &athlete.token, &second_id).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert!(
        !file_held().await,
        "the file goes with its last session, the other file's rows aside"
    );
}

/// A file declaring far more fields than any workout records is refused
/// before it is decoded whole, and the refusal names why: 8 MiB of one
/// decoded to about 1 GiB, an instance's whole memory.
#[tokio::test]
async fn a_file_declaring_more_fields_than_a_workout_is_refused_and_nothing_is_kept() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "upload-bloated").await;
    let file = bloated_file(yesterday_morning(), 8 * 1024 * 1024);
    assert!(file.len() < MAX_UPLOAD_BYTES, "under the body limit");

    let (status, body) = upload(&resources, &athlete.token, file.clone()).await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let message = body.to_string();
    assert!(
        message.contains(&FitFileError::TooLarge.to_string()),
        "the refusal names the limit: {body}"
    );
    assert!(
        !kept_file(&resources, &athlete, &file).await,
        "a refused upload keeps nothing"
    );
}

/// Whether the athlete's upload of `file` is kept.
async fn kept_file(resources: &Arc<ServerContext>, athlete: &Athlete, file: &[u8]) -> bool {
    resources
        .common
        .originals
        .get_upload(&athlete.tenant, athlete.user_id, &file_sha256(file))
        .await
        .unwrap()
        .is_some()
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

// ============================================================================
// The originals store (Athlete Data Consolidation, S12)
// ============================================================================

/// An originals store over a memory blob store the test can look into, and
/// the context's table of uploaded files.
fn inspectable_originals(resources: &Arc<ServerContext>) -> (Arc<MemoryBlobStore>, OriginalsStore) {
    let blobs = Arc::new(MemoryBlobStore::new());
    let shared: Arc<dyn BlobStore> = blobs.clone();
    let store = OriginalsStore::new(
        Ok(shared),
        Arc::clone(&resources.common.repos.uploaded_activity_files),
    );
    (blobs, store)
}

/// The blob key the athlete's upload of `sha` lives under.
fn upload_key(athlete: &Athlete, sha: &str) -> BlobKey {
    BlobKey::upload(
        Scope::new(athlete.tenant.as_uuid(), athlete.user_id),
        Sha256::parse_hex(sha).unwrap(),
    )
}

/// Run `sqlite_sql` or `postgres_sql`, whichever engine the context's
/// database is, and hand back what the engine answered.
async fn raw(
    resources: &Arc<ServerContext>,
    sqlite_sql: &str,
    #[cfg_attr(not(feature = "postgresql"), expect(unused_variables))] postgres_sql: &str,
) -> Result<(), sqlx::Error> {
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => sqlx::raw_sql(sqlite_sql)
            .execute(sqlite.pool())
            .await
            .map(drop),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => sqlx::raw_sql(postgres_sql)
            .execute(postgres.pool())
            .await
            .map(drop),
    }
}

/// A row of `uploaded_activity_files` as the table held it before the
/// drop: imported, holding `bytes` in `file_bytes` or none.
async fn insert_file_row(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    sha: &str,
    bytes: Option<&[u8]>,
) {
    const SQL: &str = "INSERT INTO uploaded_activity_files \
         (tenant_id, user_id, file_sha256, byte_size, file_bytes, uploaded_at, origin) \
         VALUES ($1, $2, $3, 4, $4, $5, 'import')";
    // One statement, bound per engine: a sqlx query is typed by its driver.
    macro_rules! insert {
        ($pool:expr) => {
            sqlx::query(SQL)
                .bind(athlete.tenant.to_string())
                .bind(athlete.user_id.to_string())
                .bind(sha.to_owned())
                .bind(bytes.map(<[u8]>::to_vec))
                .bind(Utc::now())
                .execute($pool)
                .await
                .unwrap()
                .rows_affected()
        };
    }
    let written = match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => insert!(sqlite.pool()),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => insert!(postgres.pool()),
    };
    assert_eq!(written, 1, "bug: the test's file row {sha} was not written");
}

/// How many rows of `uploaded_activity_files` hold bytes, read from the
/// table itself.
async fn rows_holding_bytes(resources: &Arc<ServerContext>) -> i64 {
    const SQL: &str =
        "SELECT count(*) AS held FROM uploaded_activity_files WHERE file_bytes IS NOT NULL";
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => sqlx::query_scalar(SQL)
            .fetch_one(sqlite.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => sqlx::query_scalar(SQL)
            .fetch_one(postgres.pool())
            .await
            .unwrap(),
    }
}

/// An upload's bytes live in the blob store under its own `upload` key —
/// never an import's `original` key — and the table names the file without
/// holding it; deleting it removes the bytes, then the row.
#[tokio::test]
async fn an_upload_is_kept_in_the_blob_store_with_a_row_naming_it() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "originals-put").await;
    let (blobs, originals) = inspectable_originals(&resources);
    let file = FitWorkout::ride(yesterday_morning(), 600).encode();
    let sha = file_sha256(&file);

    assert!(
        originals
            .put_upload(
                &athlete.tenant,
                athlete.user_id,
                &sha,
                Bytes::from(file.clone())
            )
            .await
            .unwrap(),
        "bug: a new file is new to the athlete"
    );
    assert!(
        !originals
            .put_upload(
                &athlete.tenant,
                athlete.user_id,
                &sha,
                Bytes::from(file.clone())
            )
            .await
            .unwrap(),
        "bug: the same file again is one file"
    );
    assert_eq!(
        blobs
            .get(&upload_key(&athlete, &sha))
            .await
            .unwrap()
            .as_deref(),
        Some(file.as_slice()),
        "bug: the bytes live in the blob store under the upload key"
    );
    let scope = Scope::new(athlete.tenant.as_uuid(), athlete.user_id);
    assert!(
        blobs
            .get(&BlobKey::original(scope, Sha256::parse_hex(&sha).unwrap()))
            .await
            .unwrap()
            .is_none(),
        "bug: an upload is kept apart from an import's originals"
    );
    assert_eq!(
        originals
            .get_upload(&athlete.tenant, athlete.user_id, &sha)
            .await
            .unwrap()
            .as_deref(),
        Some(file.as_slice()),
        "bug: the upload reads back"
    );
    let stranger = seed_athlete(&resources, "originals-stranger").await;
    assert!(
        originals
            .get_upload(&stranger.tenant, stranger.user_id, &sha)
            .await
            .unwrap()
            .is_none(),
        "bug: another athlete holds no such file, whatever its hash"
    );

    assert!(
        originals
            .delete_upload(&athlete.tenant, athlete.user_id, &sha)
            .await
            .unwrap(),
        "bug: the athlete held the file"
    );
    assert!(
        blobs
            .get(&upload_key(&athlete, &sha))
            .await
            .unwrap()
            .is_none(),
        "bug: a deleted upload leaves its bytes in the blob store"
    );
    assert!(
        !resources
            .common
            .repos
            .uploaded_activity_files
            .holds_uploaded_file(&athlete.tenant, athlete.user_id, &sha)
            .await
            .unwrap(),
        "bug: a deleted upload leaves its row"
    );
}

/// Bytes that are not the file the hash names are never kept under it, and
/// no row names them.
#[tokio::test]
async fn bytes_that_are_not_their_hash_are_refused_and_nothing_is_kept() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "originals-mismatch").await;
    let (blobs, originals) = inspectable_originals(&resources);
    let named = FitWorkout::ride(yesterday_morning(), 600).encode();
    let sha = file_sha256(&named);
    let other = FitWorkout::ride(yesterday_morning(), 900).encode();

    assert!(
        originals
            .put_upload(&athlete.tenant, athlete.user_id, &sha, Bytes::from(other))
            .await
            .is_err(),
        "bug: bytes another hash names were accepted"
    );
    assert!(
        blobs
            .get(&upload_key(&athlete, &sha))
            .await
            .unwrap()
            .is_none(),
        "bug: the wrong bytes were left under the hash"
    );
    assert!(
        !resources
            .common
            .repos
            .uploaded_activity_files
            .holds_uploaded_file(&athlete.tenant, athlete.user_id, &sha)
            .await
            .unwrap(),
        "bug: a row names a file the store does not hold"
    );
}

/// Where no blob store is placed (a Postgres deployment whose bucket is not
/// applied), an upload is refused as unavailable before anything is kept: no
/// row, no activity. A file a row already names cannot be read, and its
/// delete fails and leaves the row, so the retry finishes it once a store is
/// placed — no bytes are ever kept in the table.
#[tokio::test]
async fn without_a_blob_store_an_upload_is_refused_and_nothing_is_kept() {
    let resources = common::create_test_server_resources_with_blobs(Err(
        "BARRAGE_GCS_BUCKET is unset".to_owned(),
    ))
    .await
    .unwrap();
    let athlete = seed_athlete(&resources, "originals-unplaced").await;
    let file = FitWorkout::ride(yesterday_morning(), 600).encode();
    let sha = file_sha256(&file);

    let (status, body) = upload(&resources, &athlete.token, file).await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "bug: an upload with no blob store to keep it must be refused: {body}"
    );
    let files = &resources.common.repos.uploaded_activity_files;
    assert!(
        !files
            .holds_uploaded_file(&athlete.tenant, athlete.user_id, &sha)
            .await
            .unwrap(),
        "bug: a refused upload left a row naming bytes no store holds"
    );
    assert!(
        !files
            .file_backs_cached_activity(&athlete.tenant, athlete.user_id, &sha)
            .await
            .unwrap(),
        "bug: a refused upload left an activity behind"
    );

    // A row written while a store was placed, read where none is.
    files
        .record_uploaded_file(&athlete.tenant, athlete.user_id, &sha, 10, UPLOAD)
        .await
        .unwrap();
    let originals = Arc::clone(&resources.common.originals);
    assert!(
        originals
            .get_upload(&athlete.tenant, athlete.user_id, &sha)
            .await
            .is_err(),
        "bug: a file whose store is not placed must not read as absent"
    );
    assert!(
        originals
            .delete_upload(&athlete.tenant, athlete.user_id, &sha)
            .await
            .is_err(),
        "bug: a delete that cannot reach the bytes must fail"
    );
    assert!(
        files
            .holds_uploaded_file(&athlete.tenant, athlete.user_id, &sha)
            .await
            .unwrap(),
        "bug: a failed delete dropped the row its retry needs"
    );
}

/// A blob store a test scripts: deletes that fail a set number of times.
struct ScriptedBlobs {
    inner: MemoryBlobStore,
    failing_deletes: AtomicUsize,
}

impl ScriptedBlobs {
    fn new() -> Self {
        Self {
            inner: MemoryBlobStore::new(),
            failing_deletes: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl BlobStore for ScriptedBlobs {
    async fn put_stream(
        &self,
        key: &BlobKey,
        body: BoxStream<'_, Result<Bytes, BarrageError>>,
        meta: &BlobMeta,
    ) -> Result<BlobStat, BarrageError> {
        self.inner.put_stream(key, body, meta).await
    }

    async fn put(
        &self,
        key: &BlobKey,
        bytes: Bytes,
        meta: &BlobMeta,
    ) -> Result<BlobStat, BarrageError> {
        self.inner.put(key, bytes, meta).await
    }

    async fn open_upload(
        &self,
        key: &BlobKey,
        meta: &BlobMeta,
        length: u64,
        origin: Option<&str>,
    ) -> Result<UploadTarget, BarrageError> {
        self.inner.open_upload(key, meta, length, origin).await
    }

    async fn get(&self, key: &BlobKey) -> Result<Option<Bytes>, BarrageError> {
        self.inner.get(key).await
    }

    async fn get_range(&self, key: &BlobKey, range: Range<u64>) -> Result<Bytes, BarrageError> {
        self.inner.get_range(key, range).await
    }

    async fn size(&self, key: &BlobKey) -> Result<Option<u64>, BarrageError> {
        self.inner.size(key).await
    }

    async fn delete(&self, key: &BlobKey) -> Result<(), BarrageError> {
        let failing = self.failing_deletes.load(Ordering::SeqCst);
        if failing > 0
            && self
                .failing_deletes
                .compare_exchange(failing, failing - 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            return Err(BarrageError::Blob("scripted delete failure".to_owned()));
        }
        self.inner.delete(key).await
    }

    async fn delete_prefix_page(
        &self,
        prefix: &BlobPrefix,
        token: Option<PageToken>,
    ) -> Result<DeletePage, BarrageError> {
        self.inner.delete_prefix_page(prefix, token).await
    }
}

/// A delete whose file could not be removed from the blob store answers an
/// error, and the retry finishes it: the file's bytes and row both go,
/// although the activity went the first time.
#[tokio::test]
async fn a_delete_that_failed_on_the_file_is_finished_by_its_retry() {
    let blobs = Arc::new(ScriptedBlobs::new());
    let shared: Arc<dyn BlobStore> = blobs.clone();
    let resources = common::create_test_server_resources_with_blobs(Ok(shared))
        .await
        .unwrap();
    let athlete = seed_athlete(&resources, "originals-retry").await;
    let file = FitWorkout::ride(yesterday_morning(), 600).encode();
    let sha = file_sha256(&file);
    let (status, body) = upload(&resources, &athlete.token, file).await;
    assert_eq!(status, StatusCode::CREATED, "upload: {body}");

    blobs.failing_deletes.store(1, Ordering::SeqCst);
    let id = format!("{sha}-0");
    let (status, _) = delete(&resources, &athlete.token, &id).await;
    assert!(
        status.is_server_error(),
        "bug: a blob store failure must surface, got {status}"
    );
    let (status, body) = delete(&resources, &athlete.token, &id).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "bug: the retry must finish the delete, not answer not found: {body}"
    );
    assert!(
        blobs
            .get(&upload_key(&athlete, &sha))
            .await
            .unwrap()
            .is_none(),
        "bug: the file's bytes outlived the retried delete"
    );
    assert!(
        !resources
            .common
            .repos
            .uploaded_activity_files
            .holds_uploaded_file(&athlete.tenant, athlete.user_id, &sha)
            .await
            .unwrap(),
        "bug: the file's row outlived the retried delete"
    );
    let (status, _) = delete(&resources, &athlete.token, &id).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "bug: once finished, the delete is not found"
    );
}
/// S12 phase 2 (carnet#1309): the migration that drops
/// `uploaded_activity_files.file_bytes` once every upload's bytes live in
/// the blob store. Read as a file because the migration is the data under
/// test: the database already ran it, so the test replays it, whole, over a
/// table given the column back.
const SQLITE_DROP_FILE_BYTES: &str =
    include_str!("../../../../migrations/20261102120000_drop_uploaded_file_bytes.sql");
const POSTGRES_DROP_FILE_BYTES: &str =
    include_str!("../../../../migrations_pg/20261102120000_drop_uploaded_file_bytes.sql");

/// The drop refuses while any row still holds bytes, leaving the column and
/// the bytes as they were, and its refusal names what is held and the
/// remedy, since no code is left to drain a row; once none does, the column
/// goes and every row naming a file stays, with its origin.
#[tokio::test]
async fn the_file_bytes_drop_refuses_while_a_row_holds_bytes() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "originals-drop").await;
    // The databases before this migration still had the column.
    raw(
        &resources,
        "ALTER TABLE uploaded_activity_files ADD COLUMN file_bytes BLOB",
        "ALTER TABLE uploaded_activity_files ADD COLUMN file_bytes BYTEA",
    )
    .await
    .unwrap();
    let held = file_sha256(b"held");
    let drained = file_sha256(b"drained");
    // Bytes that are not the file their row names: the drain refused such a
    // row on every pass, so it would hold its bytes for good.
    let corrupt = file_sha256(b"gone");
    insert_file_row(&resources, &athlete, &held, Some(b"held")).await;
    insert_file_row(&resources, &athlete, &drained, None).await;
    insert_file_row(&resources, &athlete, &corrupt, Some(b"lost")).await;

    let refusal = raw(&resources, SQLITE_DROP_FILE_BYTES, POSTGRES_DROP_FILE_BYTES)
        .await
        .expect_err("bug: the column was dropped while a row still held bytes")
        .to_string();
    for remedy in ["carnet#1309", "operator decision"] {
        assert!(
            refusal.contains(remedy),
            "bug: the refusal does not name its remedy ({remedy}): {refusal}"
        );
    }
    #[cfg(feature = "postgresql")]
    if matches!(
        resources.agent.database.backend(),
        DatabaseBackend::PostgreSQL(_)
    ) {
        assert!(
            refusal.contains("2 uploaded file(s)") && refusal.contains("1 of them"),
            "bug: the refusal does not count the held and the unrecoverable files: {refusal}"
        );
    }
    assert_eq!(
        rows_holding_bytes(&resources).await,
        2,
        "bug: the refused drop lost the bytes a row held"
    );

    let clear = "UPDATE uploaded_activity_files SET file_bytes = NULL";
    raw(&resources, clear, clear).await.unwrap();
    raw(&resources, SQLITE_DROP_FILE_BYTES, POSTGRES_DROP_FILE_BYTES)
        .await
        .unwrap();
    let select = "SELECT file_bytes FROM uploaded_activity_files";
    assert!(
        raw(&resources, select, select).await.is_err(),
        "bug: uploaded_activity_files.file_bytes is still there"
    );
    let files = &resources.common.repos.uploaded_activity_files;
    for sha in [&held, &drained, &corrupt] {
        assert!(
            files
                .holds_uploaded_file(&athlete.tenant, athlete.user_id, sha)
                .await
                .unwrap(),
            "bug: the drop lost the row naming {sha}"
        );
    }
    let origin = "SELECT 1 FROM uploaded_activity_files WHERE origin = 'import'";
    raw(&resources, origin, origin).await.unwrap();
    assert!(
        files
            .record_uploaded_file(
                &athlete.tenant,
                athlete.user_id,
                &file_sha256(b"new"),
                3,
                UPLOAD
            )
            .await
            .unwrap(),
        "bug: a file is recorded after the drop"
    );
}
