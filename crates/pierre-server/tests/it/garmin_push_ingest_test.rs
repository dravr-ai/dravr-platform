// ABOUTME: Pins the Garmin push webhook end to end: authentication, the stored batch, the ingest into every store
// ABOUTME: Upserts on resend, sleep tentative→final, deregistration and permission purges, the inbox sweep and retries
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin push ingest suite (carnet#1155).
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
//
// Every payload here is synthetic: ids, user ids and readings are made up in
// the shape the Garmin push summaries carry.
#![cfg(all(feature = "health-sync", feature = "provider-garmin"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use secrecy::SecretString;

use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::Router;
use chrono::{Duration as ChronoDuration, NaiveTime, Utc};
use dravr_cageux::visitor::NormalizedPowerCalculator;
use dravr_riviere::{SeriesType, TimeRange};
use futures_util::stream::poll_fn;
use pierre_core::models::{
    Activity, ConnectionType, DataSource, DeviceType, StoredRecoveryMetrics, TenantId,
    UserOAuthToken,
};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::{GarminPushInboxRow, InboxClaim, JobLease, INBOX_PENDING};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::garmin_webhook::process_garmin_inbox;
use pierre_mcp_server::routes::webhooks::WebhookRoutes;
use pierre_mcp_server::services::activity_upload::MAX_UPLOAD_BYTES;
use pierre_providers::upload_provider::file_sha256;
use pierre_test_support::fit::{power as fit_power, FitWorkout};
use pierre_tool_runtime::implementations::activity_summary::ActivitySummary;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::time::sleep;
use tower::ServiceExt;
use uuid::Uuid;

/// The path token the developer portal's URLs carry.
const PATH_TOKEN: &str = "garmin-path-token-7f3c2a9e41d84b06";

/// Dravr's Garmin client id, as Garmin names it in its header.
const CLIENT_ID: &str = "garmin-client-test-0001";

/// The Garmin user id the seeded athlete's token captured.
const GARMIN_USER: &str = "garmin-user-a1b2c3";

/// A start inside the activity cache's retention window.
fn recent_start() -> i64 {
    (Utc::now() - ChronoDuration::days(2)).timestamp()
}

/// A server context with the Garmin push path token and client configured,
/// whose Garmin API origin is `api_base`.
async fn context(api_base: Option<&str>) -> Arc<ServerContext> {
    let mut config = common::test_server_config();
    common::set_server_app(&mut config, "garmin", CLIENT_ID, "garmin-test-secret");
    config.webhooks.garmin_token = Some(SecretString::from(PATH_TOKEN));
    if let Some(base) = api_base {
        config.external_services.garmin_api.base_url = format!("{base}/wellness-api/rest");
    }
    common::create_test_server_resources_with_config(config)
        .await
        .unwrap()
}

/// Seed an athlete connected through the Garmin API: a `garmin` token that
/// captured `GARMIN_USER`, and its connection row.
async fn seed_athlete(resources: &ServerContext) -> (Uuid, TenantId) {
    let (user_id, _user, tenant_id) = common::create_test_user_with_plan(
        &resources.agent.database,
        &format!("garmin-push-{}@example.com", Uuid::new_v4()),
        "starter",
    )
    .await
    .unwrap();
    store_token(resources, user_id, tenant_id, "garmin", Some(GARMIN_USER)).await;
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant_id, "garmin", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    (user_id, tenant_id)
}

async fn store_token(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    provider_user_id: Option<&str>,
) {
    let now = Utc::now();
    let token = UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id,
        tenant_id: tenant_id.to_string(),
        provider: provider.to_owned(),
        access_token: SecretString::from(format!("{provider}_access_token_0123456789")),
        refresh_token: Some(SecretString::from(format!("{provider}_refresh"))),
        token_type: "Bearer".to_owned(),
        expires_at: Some(now + ChronoDuration::hours(2)),
        scope: None,
        provider_user_id: provider_user_id.map(str::to_owned),
        oauth_app_client_id: None,
        created_at: now,
        updated_at: now,
    };
    resources
        .common
        .repos
        .oauth_tokens
        .upsert_token(&token)
        .await
        .unwrap();
}

/// POST `body` to the Garmin push URL of `summary_type`, under `token`,
/// naming `client` in the header when given.
async fn post_as(
    resources: &Arc<ServerContext>,
    token: &str,
    summary_type: &str,
    client: Option<&str>,
    body: &Value,
) -> StatusCode {
    let mut request = Request::builder()
        .method("POST")
        .uri(format!("/webhooks/garmin/{token}/{summary_type}"))
        .header("content-type", "application/json");
    if let Some(client) = client {
        request = request.header("garmin-client-id", client);
    }
    WebhookRoutes::routes(Arc::clone(resources))
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap()
        .status()
}

/// POST a genuine push of `summary_type` and wait for its ingest.
async fn push(resources: &Arc<ServerContext>, summary_type: &str, records: Value) {
    let body = json!({ summary_type: records });
    let status = post_as(resources, PATH_TOKEN, summary_type, Some(CLIENT_ID), &body).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a genuine {summary_type} push is accepted"
    );
    await_spawned_turns(resources).await;
}

/// Wait for every turn the webhook spawned on the drain tracker to finish.
async fn await_spawned_turns(resources: &ServerContext) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !resources.common.turns.is_empty() {
        assert!(
            Instant::now() < deadline,
            "webhook-spawned ingest did not finish within 15s"
        );
        sleep(Duration::from_millis(25)).await;
    }
}

fn activity(activity_id: &str, name: &str, start: i64, duration: i64) -> Value {
    json!({
        "userId": GARMIN_USER,
        "summaryId": activity_id,
        "activityId": activity_id.parse::<u64>().unwrap(),
        "activityName": name,
        "activityType": "ROAD_BIKING",
        "startTimeInSeconds": start,
        "startTimeOffsetInSeconds": -14_400,
        "durationInSeconds": duration,
        "distanceInMeters": 28_400.0,
        "averageHeartRateInBeatsPerMinute": 139,
        "deviceName": "Edge 840",
    })
}

async fn cached_garmin(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: &TenantId,
) -> Vec<Activity> {
    resources
        .common
        .repos
        .activity_cache
        .get_cached_activities(
            user_id,
            tenant_id,
            Some("garmin"),
            Utc::now() - ChronoDuration::days(30),
            Utc::now() + ChronoDuration::days(1),
            100,
        )
        .await
        .unwrap()
}

async fn garmin_recovery(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: &TenantId,
) -> Vec<StoredRecoveryMetrics> {
    resources
        .common
        .repos
        .recovery
        .get_recovery_metrics(
            user_id,
            tenant_id,
            Utc::now() - ChronoDuration::days(30),
            Utc::now() + ChronoDuration::days(1),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn a_push_with_a_wrong_path_token_or_client_is_refused() {
    let resources = context(None).await;
    let body = json!({ "activities": [activity("910001", "Ride", recent_start(), 3_600)] });

    assert_eq!(
        post_as(
            &resources,
            "not-the-token",
            "activities",
            Some(CLIENT_ID),
            &body
        )
        .await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post_as(
            &resources,
            PATH_TOKEN,
            "activities",
            Some("another-client"),
            &body
        )
        .await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post_as(&resources, PATH_TOKEN, "activities", None, &body).await,
        StatusCode::FORBIDDEN,
        "a push without the client header is refused"
    );
    assert_eq!(
        post_as(&resources, PATH_TOKEN, "sleeps", Some(CLIENT_ID), &body).await,
        StatusCode::BAD_REQUEST,
        "a list of one type pushed to another type's URL is refused"
    );
    assert_eq!(
        post_as(
            &resources,
            PATH_TOKEN,
            "epochs",
            Some(CLIENT_ID),
            &json!({ "epochs": [] })
        )
        .await,
        StatusCode::NOT_FOUND,
        "a summary type Dravr does not ingest is not found"
    );
    assert!(
        resources.common.turns.is_empty(),
        "no refused push started an ingest"
    );
}

#[tokio::test]
async fn a_push_with_a_wrong_token_is_refused_before_its_body_is_read() {
    let resources = context(None).await;
    let polled = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&polled);
    // A body that counts every read: a refusal decided from the request's
    // head never reads it, however large it claims to be.
    let body = Body::from_stream(poll_fn(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
        Poll::Ready(Some(Ok::<_, io::Error>(Bytes::from(vec![
            b'x';
            1024 * 1024
        ]))))
    }));
    let request = Request::builder()
        .method("POST")
        .uri("/webhooks/garmin/not-the-token/activities")
        .header("content-type", "application/json")
        .header("garmin-client-id", CLIENT_ID)
        .body(body)
        .unwrap();
    let status = WebhookRoutes::routes(Arc::clone(&resources))
        .oneshot(request)
        .await
        .unwrap()
        .status();
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        polled.load(Ordering::SeqCst),
        0,
        "an unauthenticated push's body is never read"
    );
}

#[tokio::test]
async fn a_push_is_refused_while_the_path_token_is_unset() {
    let mut config = common::test_server_config();
    common::set_server_app(&mut config, "garmin", CLIENT_ID, "garmin-test-secret");
    config.webhooks.garmin_token = None;
    let resources = common::create_test_server_resources_with_config(config)
        .await
        .unwrap();
    let body = json!({ "activities": [] });
    assert_eq!(
        post_as(&resources, PATH_TOKEN, "activities", Some(CLIENT_ID), &body).await,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn activities_land_in_the_cache_and_a_resend_replaces_the_row() {
    let resources = context(None).await;
    let (user_id, tenant_id) = seed_athlete(&resources).await;
    let start = recent_start();

    push(
        &resources,
        "activities",
        json!([activity("920001", "Morning ride", start, 3_600)]),
    )
    .await;
    let cached = cached_garmin(&resources, user_id, &tenant_id).await;
    assert_eq!(cached.len(), 1);
    assert_eq!(cached[0].id(), "920001");
    assert_eq!(cached[0].name(), "Morning ride");
    assert_eq!(cached[0].device_name(), Some("Garmin Edge 840"));
    assert!(
        resources
            .common
            .repos
            .activity_cache
            .latest_activity_sync(user_id, &tenant_id, "garmin")
            .await
            .unwrap()
            .is_some(),
        "the push is the provider's freshness"
    );

    push(
        &resources,
        "activities",
        json!([activity("920001", "Morning ride, renamed", start, 3_600)]),
    )
    .await;
    let cached = cached_garmin(&resources, user_id, &tenant_id).await;
    assert_eq!(cached.len(), 1, "a re-sent summary upserts the same row");
    assert_eq!(cached[0].name(), "Morning ride, renamed");
}

#[tokio::test]
async fn manual_and_multisport_parent_activities_are_not_kept() {
    let resources = context(None).await;
    let (user_id, tenant_id) = seed_athlete(&resources).await;
    let start = recent_start();
    let mut manual = activity("930001", "Typed in", start, 1_800);
    manual["manual"] = json!(true);
    let mut parent = activity("930002", "Triathlon", start, 7_200);
    parent["isParent"] = json!(true);
    let mut leg = activity("930003", "Bike leg", start + 1_200, 3_600);
    leg["parentSummaryId"] = json!("930002");

    push(&resources, "activities", json!([manual, parent, leg])).await;
    let ids: Vec<String> = cached_garmin(&resources, user_id, &tenant_id)
        .await
        .iter()
        .map(|a| a.id().to_owned())
        .collect();
    assert_eq!(ids, vec!["930003".to_owned()]);
}

#[tokio::test]
async fn a_push_for_an_unknown_garmin_user_is_dropped() {
    let resources = context(None).await;
    let (user_id, tenant_id) = seed_athlete(&resources).await;
    let mut stranger = activity("940001", "Not ours", recent_start(), 3_600);
    stranger["userId"] = json!("garmin-user-nobody-here");

    push(&resources, "activities", json!([stranger])).await;
    assert!(cached_garmin(&resources, user_id, &tenant_id)
        .await
        .is_empty());
}

#[tokio::test]
async fn activity_details_store_laps_and_samples_beside_the_row() {
    let resources = context(None).await;
    let (user_id, tenant_id) = seed_athlete(&resources).await;
    let start = recent_start();
    let samples: Vec<Value> = (0..60)
        .map(|i| {
            json!({
                "startTimeInSeconds": start + i * 10,
                "heartRate": 120 + i,
                "totalDistanceInMeters": f64::from(u32::try_from(i).unwrap()) * 40.0,
                "latitudeInDegree": 45.5,
                "longitudeInDegree": -73.6,
            })
        })
        .collect();
    let details = json!([{
        "userId": GARMIN_USER,
        "summaryId": "950001-detail",
        "activityId": 950_001,
        "summary": activity("950001", "Intervals", start, 600),
        "samples": samples,
        "laps": [{ "startTimeInSeconds": start }, { "startTimeInSeconds": start + 300 }],
    }]);

    push(&resources, "activityDetails", details).await;
    let cached = cached_garmin(&resources, user_id, &tenant_id).await;
    assert_eq!(cached.len(), 1, "the details' own summary creates the row");
    assert_eq!(
        cached[0].laps().map(Vec::len),
        Some(2),
        "laps stand beside the row"
    );
    let streams = resources
        .common
        .repos
        .activity_streams
        .get_activity_streams(&tenant_id, user_id, "garmin", "950001")
        .await
        .unwrap()
        .expect("the samples are stored");
    assert_eq!(streams.timestamps.len(), 60);
    assert_eq!(streams.heart_rate.as_ref().map(Vec::len), Some(60));
}

/// Garmin `activityDetails` samples a second apart from `start`, recording
/// 150 W and 250 W in 30-second blocks, as `FitWorkout`'s power does.
fn powered_samples(start: i64, seconds: u32) -> Vec<Value> {
    (0..seconds)
        .map(|second| {
            json!({
                "startTimeInSeconds": start + i64::from(second),
                "heartRate": 140,
                "powerInWatts": fit_power(second),
            })
        })
        .collect()
}

/// The normalized power cageux gives a ride of `seconds` of [`fit_power`].
fn expected_np(seconds: u32) -> u32 {
    let stream: Vec<Option<u32>> = (0..seconds)
        .map(|second| Some(u32::from(fit_power(second))))
        .collect();
    let np = NormalizedPowerCalculator::from_power_stream(&stream).unwrap();
    // A whole number of watts, as every provider maps it.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let np = np.round() as u32;
    np
}

#[tokio::test]
async fn a_pushed_rides_power_comes_from_its_samples_and_outlives_a_resent_summary() {
    let resources = context(None).await;
    let (user_id, tenant_id) = seed_athlete(&resources).await;
    let start = recent_start();
    let ride = activity("955001", "Tempo", start, 600);
    let details = json!([{
        "userId": GARMIN_USER,
        "summaryId": "955001-detail",
        "activityId": 955_001,
        "summary": ride,
        "samples": powered_samples(start, 600),
    }]);

    push(&resources, "activityDetails", details).await;
    let np = expected_np(600);
    let check = |label: &str, cached: &[Activity]| {
        let row = cached
            .first()
            .unwrap_or_else(|| panic!("{label}: the ride is cached"));
        assert_eq!(
            row.average_power(),
            Some(200),
            "bug: {label}: the samples' mean power"
        );
        assert_eq!(
            row.max_power(),
            Some(250),
            "bug: {label}: the samples' highest power"
        );
        assert_eq!(
            row.normalized_power(),
            Some(np),
            "bug: {label}: a pushed ride's NP is cageux's over its samples"
        );
        let summary = ActivitySummary::from(row);
        assert_eq!(
            summary.normalized_power,
            Some(np),
            "bug: {label}: the agent reads the ride's NP"
        );
    };
    check(
        "after its details",
        &cached_garmin(&resources, user_id, &tenant_id).await,
    );

    // Garmin re-sends the summary, which carries no power: the row is
    // rewritten from it, and the stored samples give the power back.
    push(&resources, "activities", json!([ride])).await;
    check(
        "after a re-sent summary",
        &cached_garmin(&resources, user_id, &tenant_id).await,
    );
}

#[tokio::test]
async fn a_tentative_sleep_is_replaced_by_its_final_record() {
    let resources = context(None).await;
    let (user_id, tenant_id) = seed_athlete(&resources).await;
    let start = recent_start();
    let sleep_record = |validation: &str, asleep: i64, score: u32| {
        json!({
            "userId": GARMIN_USER,
            "summaryId": format!("sleep-{validation}"),
            "calendarDate": "2026-10-20",
            "startTimeInSeconds": start,
            "durationInSeconds": asleep,
            "deepSleepDurationInSeconds": asleep / 4,
            "lightSleepDurationInSeconds": asleep / 2,
            "remSleepInSeconds": asleep / 4,
            "awakeDurationInSeconds": 600,
            "validation": validation,
            "overallSleepScore": { "value": score },
        })
    };

    push(
        &resources,
        "sleeps",
        json!([sleep_record("AUTO_TENTATIVE", 14_400, 61)]),
    )
    .await;
    push(
        &resources,
        "sleeps",
        json!([sleep_record("ENHANCED_FINAL", 25_200, 84)]),
    )
    .await;

    let sessions = resources
        .common
        .repos
        .sleep
        .get_sleep_sessions(
            user_id,
            &tenant_id,
            Utc::now() - ChronoDuration::days(30),
            Utc::now() + ChronoDuration::days(1),
        )
        .await
        .unwrap();
    assert_eq!(
        sessions.len(),
        1,
        "the final record replaces the tentative one"
    );
    assert_eq!(sessions[0].total_sleep_seconds, Some(25_200));
    assert_eq!(sessions[0].sleep_score, Some(84));
}

#[tokio::test]
async fn dailies_hrv_and_body_pushes_merge_on_their_days_rows() {
    let resources = context(None).await;
    let (user_id, tenant_id) = seed_athlete(&resources).await;
    let date = (Utc::now() - ChronoDuration::days(1)).date_naive();
    let calendar = date.format("%Y-%m-%d").to_string();
    let day_start = date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();

    push(
        &resources,
        "dailies",
        json!([{
            "userId": GARMIN_USER,
            "calendarDate": calendar,
            "startTimeInSeconds": day_start,
            "restingHeartRateInBeatsPerMinute": 47,
            "averageStressLevel": 28,
            "timeOffsetHeartRateSamples": { "15": 58, "30": 60 },
        }]),
    )
    .await;
    push(
        &resources,
        "hrv",
        json!([{
            "userId": GARMIN_USER,
            "calendarDate": calendar,
            "startTimeInSeconds": day_start,
            "lastNightAvg": 63,
            "hrvValues": { "300": 59, "600": 66 },
        }]),
    )
    .await;
    push(
        &resources,
        "bodyComps",
        json!([{
            "userId": GARMIN_USER,
            "measurementTimeInSeconds": day_start + 25_000,
            "measurementTimeOffsetInSeconds": 0,
            "weightInGrams": 70_800,
            "bodyFatInPercent": 13.4,
        }]),
    )
    .await;
    push(
        &resources,
        "userMetrics",
        json!([{ "userId": GARMIN_USER, "calendarDate": calendar, "vo2Max": 56.0 }]),
    )
    .await;

    let recovery = garmin_recovery(&resources, user_id, &tenant_id).await;
    assert_eq!(recovery.len(), 1, "one recovery row for the day");
    assert_eq!(
        recovery[0].resting_heart_rate,
        Some(47),
        "the dailies reading stays"
    );
    assert_eq!(recovery[0].stress_score, Some(28));
    assert_eq!(
        recovery[0].hrv_rmssd,
        Some(63.0),
        "the hrv reading joins it"
    );

    let body = resources
        .common
        .repos
        .health_snapshots
        .get_health_snapshots(
            user_id,
            &tenant_id,
            Utc::now() - ChronoDuration::days(30),
            Utc::now() + ChronoDuration::days(1),
        )
        .await
        .unwrap();
    assert_eq!(body.len(), 1, "one body row for the day");
    assert_eq!(body[0].weight_kg, Some(70.8));
    assert_eq!(body[0].vo2_max, Some(56.0), "VO2max joins the weighed row");
}

#[tokio::test]
async fn a_deregistration_deletes_the_garmin_api_connection_but_not_the_usr_pwd_one() {
    let resources = context(None).await;
    let (user_id, tenant_id) = seed_athlete(&resources).await;
    store_token(&resources, user_id, tenant_id, "sciotte_garmin", None).await;
    push(
        &resources,
        "activities",
        json!([activity("960001", "Before leaving", recent_start(), 3_600)]),
    )
    .await;
    assert_eq!(
        cached_garmin(&resources, user_id, &tenant_id).await.len(),
        1
    );

    push(
        &resources,
        "deregistrations",
        json!([{ "userId": GARMIN_USER }]),
    )
    .await;

    let tokens = resources
        .common
        .repos
        .oauth_tokens
        .get_tokens(user_id, Some(tenant_id))
        .await
        .unwrap();
    let providers: Vec<&str> = tokens.iter().map(|t| t.provider.as_str()).collect();
    assert!(
        !providers.contains(&"garmin"),
        "the Garmin API token is deleted"
    );
    assert!(
        providers.contains(&"sciotte_garmin"),
        "the usr/pwd connection stays"
    );
    assert!(cached_garmin(&resources, user_id, &tenant_id)
        .await
        .is_empty());
    let purges = resources
        .common
        .repos
        .provider_data
        .list_provider_data_purges("garmin", 10)
        .await
        .unwrap();
    assert_eq!(purges.len(), 1);
    assert_eq!(purges[0].reason, "provider_revoked");
}

#[tokio::test]
async fn a_withdrawn_permission_purges_that_data_and_refuses_later_pushes_of_it() {
    let resources = context(None).await;
    let (user_id, tenant_id) = seed_athlete(&resources).await;
    let date = (Utc::now() - ChronoDuration::days(1)).date_naive();
    let calendar = date.format("%Y-%m-%d").to_string();
    let daily = json!([{
        "userId": GARMIN_USER,
        "calendarDate": calendar,
        "restingHeartRateInBeatsPerMinute": 49,
    }]);
    push(&resources, "dailies", daily.clone()).await;
    push(
        &resources,
        "activities",
        json!([activity("970001", "Kept", recent_start(), 3_600)]),
    )
    .await;
    assert_eq!(
        garmin_recovery(&resources, user_id, &tenant_id).await.len(),
        1
    );

    push(
        &resources,
        "userPermissionsChange",
        json!([{
            "userId": GARMIN_USER,
            "summaryId": "perm-1",
            "permissions": ["ACTIVITY_EXPORT", "WORKOUT_IMPORT"],
            "changeTimeInSeconds": recent_start(),
        }]),
    )
    .await;
    assert!(
        garmin_recovery(&resources, user_id, &tenant_id)
            .await
            .is_empty(),
        "health data is purged when HEALTH_EXPORT is withdrawn"
    );
    assert_eq!(
        cached_garmin(&resources, user_id, &tenant_id).await.len(),
        1,
        "activities stay while ACTIVITY_EXPORT is granted"
    );

    push(&resources, "dailies", daily).await;
    assert!(
        garmin_recovery(&resources, user_id, &tenant_id)
            .await
            .is_empty(),
        "a health push after the withdrawal is refused"
    );

    push(
        &resources,
        "userPermissionsChange",
        json!([{ "userId": GARMIN_USER, "permissions": [] }]),
    )
    .await;
    assert!(
        cached_garmin(&resources, user_id, &tenant_id)
            .await
            .is_empty(),
        "activities are purged when ACTIVITY_EXPORT is withdrawn"
    );
    let reasons: Vec<String> = resources
        .common
        .repos
        .provider_data
        .list_provider_data_purges("garmin", 10)
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.reason)
        .collect();
    assert_eq!(
        reasons,
        vec!["permission_withdrawn", "permission_withdrawn"]
    );
}

#[tokio::test]
async fn a_batch_stored_before_its_instance_died_is_run_by_the_sweep() {
    let resources = context(None).await;
    let (user_id, tenant_id) = seed_athlete(&resources).await;
    let now_ms = Utc::now().timestamp_millis();
    let row = GarminPushInboxRow {
        id: Uuid::new_v4().to_string(),
        summary_type: "activities".to_owned(),
        received_at_ms: now_ms - 60_000,
        payload: json!({ "activities": [activity("980001", "Survived", recent_start(), 3_600)] })
            .to_string(),
        status: INBOX_PENDING.to_owned(),
        attempts: 1,
        last_error: None,
        processed_at_ms: None,
        // The instance that answered 200 died: its lease has lapsed.
        leased_until_ms: now_ms - 1,
        lease_token: Some(Uuid::new_v4().to_string()),
    };
    let inbox = &resources.common.repos.garmin_push_inbox;
    inbox.record_push(&row).await.unwrap();

    assert_eq!(process_garmin_inbox(&resources).await.unwrap(), 1);
    assert_eq!(
        cached_garmin(&resources, user_id, &tenant_id).await.len(),
        1
    );
    let done = inbox.get_push(&row.id).await.unwrap().unwrap();
    assert_eq!(done.status, "done");
    assert!(done.payload.is_empty(), "a processed row keeps no payload");
    assert_eq!(
        process_garmin_inbox(&resources).await.unwrap(),
        0,
        "a done row is not run again"
    );
}

/// A Garmin API origin serving one activity file once, then 410 as Garmin
/// does; returns its base URL and the download count.
async fn mock_garmin_files(file: Vec<u8>) -> (String, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);
    let app = Router::new().route(
        "/wellness-api/rest/activityFile",
        get(move || {
            let counter = Arc::clone(&counter);
            let file = file.clone();
            async move {
                if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                    (StatusCode::OK, file)
                } else {
                    (StatusCode::GONE, Vec::new())
                }
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

/// The `origin` and `source_ref` of a stored Garmin sample set, which no
/// repository read returns.
async fn stored_stream_source(
    resources: &ServerContext,
    activity_id: &str,
) -> (String, Option<String>) {
    const SQL: &str =
        "SELECT origin, source_ref FROM activity_streams WHERE provider = 'garmin' AND activity_id = $1";
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => sqlx::query_as(SQL)
            .bind(activity_id)
            .fetch_one(sqlite.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => sqlx::query_as(SQL)
            .bind(activity_id)
            .fetch_one(postgres.pool())
            .await
            .unwrap(),
    }
}

#[tokio::test]
async fn a_long_activitys_file_waits_for_its_summary_then_is_fetched_once() {
    let start = Utc::now() - ChronoDuration::days(3);
    let file = FitWorkout::ride(start, 120).encode();
    let (base, hits) = mock_garmin_files(file.clone()).await;
    let resources = context(Some(&base)).await;
    let (user_id, tenant_id) = seed_athlete(&resources).await;
    let callback = format!("{base}/wellness-api/rest/activityFile?id=1&token=one-shot");
    let file_push = json!([{
        "userId": GARMIN_USER,
        "summaryId": "990001-file",
        "activityId": 990_001,
        "fileType": "FIT",
        "callbackURL": callback,
        "manual": false,
    }]);

    // The file arrives before its summary: its length is unknown, so the
    // batch is handed back for a later run, nothing fetched.
    push(&resources, "activityFiles", file_push).await;
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    let inbox = &resources.common.repos.garmin_push_inbox;
    let claimed = inbox
        .claim_due_pushes(InboxClaim {
            now_ms: Utc::now().timestamp_millis() + 3_600_000,
            lease_ms: 60_000,
            max_attempts: 8,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1, "the handed-back batch is due again later");
    assert_eq!(claimed[0].attempts, 2);
    assert!(claimed[0].last_error.is_some());

    // The summary of a 26-hour activity lands; the handed-back batch comes
    // due and the sweep fetches the file, once.
    push(
        &resources,
        "activities",
        json!([activity("990001", "Ultra", start.timestamp(), 26 * 3_600)]),
    )
    .await;
    let lease = JobLease {
        id: claimed[0].id.clone(),
        token: claimed[0].lease_token.clone().unwrap(),
    };
    assert!(inbox
        .retry_push(&lease, "due now", Utc::now().timestamp_millis() - 1)
        .await
        .unwrap());
    assert_eq!(process_garmin_inbox(&resources).await.unwrap(), 1);

    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "the one-shot file is fetched once"
    );
    let streams = resources
        .common
        .repos
        .activity_streams
        .get_activity_streams(&tenant_id, user_id, "garmin", "990001")
        .await
        .unwrap()
        .expect("the file's samples are stored");
    assert!(
        streams.timestamps.len() >= 100,
        "the file's two minutes of samples are stored"
    );
    assert_eq!(
        stored_stream_source(&resources, "990001").await,
        ("push".to_owned(), Some(file_sha256(&file))),
        "the samples are pushed ones and name the file they were decoded from"
    );
    assert_eq!(
        inbox
            .get_push(&claimed[0].id)
            .await
            .unwrap()
            .unwrap()
            .status,
        "done"
    );
}

#[tokio::test]
async fn a_long_rides_power_comes_from_its_file() {
    let start = Utc::now() - ChronoDuration::days(3);
    let file = FitWorkout::ride(start, 120).with_power().encode();
    let (base, _hits) = mock_garmin_files(file).await;
    let resources = context(Some(&base)).await;
    let (user_id, tenant_id) = seed_athlete(&resources).await;
    push(
        &resources,
        "activities",
        json!([activity("991001", "Ultra", start.timestamp(), 26 * 3_600)]),
    )
    .await;
    let callback = format!("{base}/wellness-api/rest/activityFile?id=1&token=one-shot");
    push(
        &resources,
        "activityFiles",
        json!([{
            "userId": GARMIN_USER,
            "summaryId": "991001-file",
            "activityId": 991_001,
            "fileType": "FIT",
            "callbackURL": callback,
            "manual": false,
        }]),
    )
    .await;

    let cached = cached_garmin(&resources, user_id, &tenant_id).await;
    let row = cached.first().expect("the ultra is cached");
    assert_eq!(
        row.normalized_power(),
        Some(expected_np(120)),
        "bug: an activity longer than the details cover takes its power from its file"
    );
    assert_eq!(
        row.max_power(),
        Some(250),
        "bug: an activity longer than the details cover takes its highest power from its file"
    );
}

#[tokio::test]
async fn a_file_whose_summary_never_lands_is_settled_not_failed_after_its_last_run() {
    let (base, hits) = mock_garmin_files(Vec::new()).await;
    let resources = context(Some(&base)).await;
    seed_athlete(&resources).await;
    // A web upload's summary is skipped by design, so its file waits on a
    // summary that will never be cached.
    let mut uploaded = activity("996001", "Uploaded", recent_start(), 3_600);
    uploaded["isWebUpload"] = json!(true);
    push(&resources, "activities", json!([uploaded])).await;
    push(
        &resources,
        "activityFiles",
        json!([{
            "userId": GARMIN_USER,
            "activityId": 996_001,
            "fileType": "FIT",
            "callbackURL": format!("{base}/wellness-api/rest/activityFile?id=3&token=t"),
        }]),
    )
    .await;

    // Run the batch up to its last attempt: six claims hand it back as due,
    // the sweep's claim is the eighth run.
    let inbox = &resources.common.repos.garmin_push_inbox;
    let mut id = String::new();
    for _ in 0..6 {
        let claimed = inbox
            .claim_due_pushes(InboxClaim {
                now_ms: Utc::now().timestamp_millis() + 86_400_000,
                lease_ms: 60_000,
                max_attempts: 8,
                limit: 10,
            })
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1);
        id.clone_from(&claimed[0].id);
        let lease = JobLease {
            id: claimed[0].id.clone(),
            token: claimed[0].lease_token.clone().unwrap(),
        };
        assert!(inbox
            .retry_push(&lease, "due now", Utc::now().timestamp_millis() - 1)
            .await
            .unwrap());
    }
    assert_eq!(process_garmin_inbox(&resources).await.unwrap(), 1);

    let settled = inbox.get_push(&id).await.unwrap().unwrap();
    assert_eq!(settled.attempts, 8);
    assert_eq!(
        settled.status, "done",
        "a waiting record settles the batch, never fails it"
    );
    assert!(
        settled.last_error.is_some(),
        "what it settled without is recorded"
    );
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_short_activitys_file_is_left_unfetched() {
    let (base, hits) = mock_garmin_files(Vec::new()).await;
    let resources = context(Some(&base)).await;
    seed_athlete(&resources).await;
    push(
        &resources,
        "activities",
        json!([activity("995001", "Short", recent_start(), 3_600)]),
    )
    .await;
    push(
        &resources,
        "activityFiles",
        json!([{
            "userId": GARMIN_USER,
            "activityId": 995_001,
            "fileType": "FIT",
            "callbackURL": format!("{base}/wellness-api/rest/activityFile?id=2&token=t"),
        }]),
    )
    .await;
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "the details cover it, so its file is never fetched"
    );
}

/// Store a recovery row and a night of sleep the way the usr/pwd
/// connection's health sync stores them: under the provider `garmin`,
/// through a data source with no marker.
async fn seed_usr_pwd_wellness(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: &TenantId,
    date: chrono::NaiveDate,
) {
    let repos = &resources.common.repos;
    let source_id = repos
        .data_sources
        .upsert_data_source(
            tenant_id,
            &DataSource {
                id: String::new(),
                user_id: user_id.to_string(),
                provider: "garmin".to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Unknown,
                original_source_name: Some("garmin".to_owned()),
            },
        )
        .await
        .unwrap();
    let row = StoredRecoveryMetrics {
        id: String::new(),
        user_id: user_id.to_string(),
        data_source_id: source_id,
        date,
        recovery_score: None,
        readiness_score: None,
        hrv_ms: None,
        hrv_rmssd: None,
        resting_heart_rate: Some(52),
        stress_score: None,
        body_battery: None,
        spo2: None,
        respiratory_rate: None,
        skin_temp_deviation: None,
        daily_strain: None,
        athlete_note: None,
        source_name: "garmin".to_owned(),
        recorded_at: date.and_time(NaiveTime::MIN).and_utc(),
    };
    repos
        .recovery
        .upsert_recovery_metrics(tenant_id, &row)
        .await
        .unwrap();
}

#[tokio::test]
async fn withdrawing_the_api_leaves_the_usr_pwd_connections_wellness_history() {
    let resources = context(None).await;
    let (user_id, tenant_id) = seed_athlete(&resources).await;
    store_token(&resources, user_id, tenant_id, "sciotte_garmin", None).await;
    let history_day = (Utc::now() - ChronoDuration::days(20)).date_naive();
    seed_usr_pwd_wellness(&resources, user_id, &tenant_id, history_day).await;
    let api_day = (Utc::now() - ChronoDuration::days(1)).date_naive();
    let daily = json!([{
        "userId": GARMIN_USER,
        "calendarDate": api_day.format("%Y-%m-%d").to_string(),
        "restingHeartRateInBeatsPerMinute": 47,
    }]);
    push(&resources, "dailies", daily.clone()).await;
    assert_eq!(
        garmin_recovery(&resources, user_id, &tenant_id).await.len(),
        2
    );

    // The first permission notice leaves health out: the API's day goes,
    // the usr/pwd connection's year of history stays.
    push(
        &resources,
        "userPermissionsChange",
        json!([{ "userId": GARMIN_USER, "permissions": ["ACTIVITY_EXPORT"] }]),
    )
    .await;
    let left = garmin_recovery(&resources, user_id, &tenant_id).await;
    assert_eq!(
        left.len(),
        1,
        "only the usr/pwd row survives the withdrawal"
    );
    assert_eq!(left[0].date, history_day);
    assert_eq!(left[0].resting_heart_rate, Some(52));

    // Health is granted again and lands, then Garmin deregisters the
    // athlete: the API's rows go with its connection, the usr/pwd history
    // stays.
    push(
        &resources,
        "userPermissionsChange",
        json!([{ "userId": GARMIN_USER, "permissions": ["ACTIVITY_EXPORT", "HEALTH_EXPORT"] }]),
    )
    .await;
    push(&resources, "dailies", daily).await;
    assert_eq!(
        garmin_recovery(&resources, user_id, &tenant_id).await.len(),
        2
    );
    push(
        &resources,
        "deregistrations",
        json!([{ "userId": GARMIN_USER }]),
    )
    .await;
    let left = garmin_recovery(&resources, user_id, &tenant_id).await;
    assert_eq!(
        left.len(),
        1,
        "only the usr/pwd row survives the deregistration"
    );
    assert_eq!(left[0].date, history_day);
    let sources = resources
        .common
        .repos
        .data_sources
        .list_data_sources(user_id, &tenant_id)
        .await
        .unwrap();
    assert_eq!(
        sources.len(),
        1,
        "the API's data source is purged with its rows"
    );
    assert_eq!(sources[0].source, None);
}

/// Store a push in the inbox as if received at `received_at_ms` by an
/// instance that died before running it.
async fn store_lapsed_push(
    resources: &ServerContext,
    summary_type: &str,
    records: Value,
    received_at_ms: i64,
) {
    let row = GarminPushInboxRow {
        id: Uuid::new_v4().to_string(),
        summary_type: summary_type.to_owned(),
        received_at_ms,
        payload: json!({ summary_type: records }).to_string(),
        status: INBOX_PENDING.to_owned(),
        attempts: 1,
        last_error: None,
        processed_at_ms: None,
        leased_until_ms: Utc::now().timestamp_millis() - 1,
        lease_token: Some(Uuid::new_v4().to_string()),
    };
    resources
        .common
        .repos
        .garmin_push_inbox
        .record_push(&row)
        .await
        .unwrap();
}

#[tokio::test]
async fn an_older_push_retried_after_a_newer_one_leaves_the_newer_values() {
    let resources = context(None).await;
    let (user_id, tenant_id) = seed_athlete(&resources).await;
    let start = recent_start();
    let night = |validation: &str, asleep: i64, score: u32| {
        json!([{
            "userId": GARMIN_USER,
            "summaryId": format!("sleep-{validation}"),
            "calendarDate": "2026-10-20",
            "startTimeInSeconds": start,
            "durationInSeconds": asleep,
            "validation": validation,
            "overallSleepScore": { "value": score },
        }])
    };

    // The final night and the renamed ride land now; the tentative night
    // and the ride's first name were received a minute earlier and only run
    // afterwards, as a retried batch does.
    push(&resources, "sleeps", night("ENHANCED_FINAL", 25_200, 84)).await;
    push(
        &resources,
        "activities",
        json!([activity("991001", "Renamed ride", start, 3_600)]),
    )
    .await;
    let a_minute_ago = Utc::now().timestamp_millis() - 60_000;
    store_lapsed_push(
        &resources,
        "sleeps",
        night("AUTO_TENTATIVE", 14_400, 61),
        a_minute_ago,
    )
    .await;
    store_lapsed_push(
        &resources,
        "activities",
        json!([activity("991001", "First name", start, 3_600)]),
        a_minute_ago,
    )
    .await;
    assert_eq!(process_garmin_inbox(&resources).await.unwrap(), 2);

    let sessions = resources
        .common
        .repos
        .sleep
        .get_sleep_sessions(
            user_id,
            &tenant_id,
            Utc::now() - ChronoDuration::days(30),
            Utc::now() + ChronoDuration::days(1),
        )
        .await
        .unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].sleep_score, Some(84), "the final night stands");
    assert_eq!(sessions[0].total_sleep_seconds, Some(25_200));
    let rides = cached_garmin(&resources, user_id, &tenant_id).await;
    assert_eq!(rides.len(), 1);
    assert_eq!(rides[0].name(), "Renamed ride", "the later summary stands");
}

#[tokio::test]
async fn a_details_batch_retried_after_a_newer_summary_still_gives_the_ride_its_power() {
    let resources = context(None).await;
    let (user_id, tenant_id) = seed_athlete(&resources).await;
    let start = recent_start();

    // The renamed summary lands now; the details, received a minute earlier,
    // only run afterwards, as a retried batch does.
    push(
        &resources,
        "activities",
        json!([activity("992001", "Renamed tempo", start, 600)]),
    )
    .await;
    store_lapsed_push(
        &resources,
        "activityDetails",
        json!([{
            "userId": GARMIN_USER,
            "summaryId": "992001-detail",
            "activityId": 992_001,
            "summary": activity("992001", "First name", start, 600),
            "samples": powered_samples(start, 600),
        }]),
        Utc::now().timestamp_millis() - 60_000,
    )
    .await;
    assert_eq!(process_garmin_inbox(&resources).await.unwrap(), 1);

    let rides = cached_garmin(&resources, user_id, &tenant_id).await;
    let ride = rides.first().expect("the ride is cached");
    assert_eq!(
        ride.name(),
        "Renamed tempo",
        "bug: the later summary still stands over the retried details' own"
    );
    assert_eq!(
        ride.normalized_power(),
        Some(expected_np(600)),
        "bug: samples landing after a newer summary give it their power"
    );
    assert_eq!(
        ride.max_power(),
        Some(250),
        "bug: the standing summary takes the samples' highest power"
    );
}

#[tokio::test]
async fn pushes_for_one_day_running_at_once_keep_every_reading() {
    let resources = context(None).await;
    let (user_id, tenant_id) = seed_athlete(&resources).await;
    let date = (Utc::now() - ChronoDuration::days(1)).date_naive();
    let calendar = date.format("%Y-%m-%d").to_string();
    let daily = json!({ "dailies": [{
        "userId": GARMIN_USER,
        "calendarDate": calendar,
        "restingHeartRateInBeatsPerMinute": 47,
        "averageStressLevel": 28,
    }]});
    let hrv = json!({ "hrv": [{
        "userId": GARMIN_USER,
        "calendarDate": calendar,
        "lastNightAvg": 63.0,
    }]});
    let skin = json!({ "skinTemp": [{
        "userId": GARMIN_USER,
        "calendarDate": calendar,
        "avgDeviationCelsius": -0.4,
    }]});
    let (a, b, c) = tokio::join!(
        post_as(&resources, PATH_TOKEN, "dailies", Some(CLIENT_ID), &daily),
        post_as(&resources, PATH_TOKEN, "hrv", Some(CLIENT_ID), &hrv),
        post_as(&resources, PATH_TOKEN, "skinTemp", Some(CLIENT_ID), &skin),
    );
    assert_eq!((a, b, c), (StatusCode::OK, StatusCode::OK, StatusCode::OK));
    await_spawned_turns(&resources).await;

    let rows = garmin_recovery(&resources, user_id, &tenant_id).await;
    assert_eq!(rows.len(), 1, "one recovery row for the day");
    assert_eq!(rows[0].resting_heart_rate, Some(47));
    assert_eq!(rows[0].stress_score, Some(28));
    assert_eq!(rows[0].hrv_rmssd, Some(63.0));
    assert_eq!(rows[0].skin_temp_deviation, Some(-0.4));
}

#[tokio::test]
async fn a_kept_file_is_stored_on_retry_without_asking_garmin_again() {
    let start = Utc::now() - ChronoDuration::days(3);
    let file = FitWorkout::ride(start, 120).encode();
    // Garmin has already served this file once: every fetch now answers 410.
    let (base, hits) = mock_garmin_files(Vec::new()).await;
    hits.store(1, Ordering::SeqCst);
    let resources = context(Some(&base)).await;
    let (user_id, tenant_id) = seed_athlete(&resources).await;
    push(
        &resources,
        "activities",
        json!([activity("992001", "Ultra", start.timestamp(), 26 * 3_600)]),
    )
    .await;
    // An earlier run downloaded the file and kept it, then failed before
    // its samples landed.
    let pushed = &resources.common.repos.pushed_data;
    pushed
        .keep_file(&tenant_id, user_id, "garmin", "992001", &file, Utc::now())
        .await
        .unwrap();

    push(
        &resources,
        "activityFiles",
        json!([{
            "userId": GARMIN_USER,
            "activityId": 992_001,
            "fileType": "FIT",
            "callbackURL": format!("{base}/wellness-api/rest/activityFile?id=4&token=t"),
        }]),
    )
    .await;

    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "the kept copy is decoded; Garmin is not asked again"
    );
    let streams = resources
        .common
        .repos
        .activity_streams
        .get_activity_streams(&tenant_id, user_id, "garmin", "992001")
        .await
        .unwrap()
        .expect("the kept file's samples are stored");
    assert!(streams.timestamps.len() >= 100);
    assert!(
        pushed
            .kept_file(&tenant_id, user_id, "garmin", "992001")
            .await
            .unwrap()
            .is_none(),
        "the kept copy is dropped once its samples and laps landed"
    );
}

/// An activity file past the cap uploads are held to is never read whole,
/// kept or decoded: the batch settles with nothing stored, and Garmin is not
/// asked again for a file it serves once.
#[tokio::test]
async fn an_oversized_activity_file_is_refused_and_nothing_is_kept() {
    let start = Utc::now() - ChronoDuration::days(3);
    let oversized = vec![0_u8; MAX_UPLOAD_BYTES + 1];
    let (base, hits) = mock_garmin_files(oversized).await;
    let resources = context(Some(&base)).await;
    let (user_id, tenant_id) = seed_athlete(&resources).await;
    push(
        &resources,
        "activities",
        json!([activity("993001", "Ultra", start.timestamp(), 26 * 3_600)]),
    )
    .await;

    push(
        &resources,
        "activityFiles",
        json!([{
            "userId": GARMIN_USER,
            "activityId": 993_001,
            "fileType": "FIT",
            "callbackURL": format!("{base}/wellness-api/rest/activityFile?id=5&token=t"),
        }]),
    )
    .await;

    assert_eq!(hits.load(Ordering::SeqCst), 1, "the file is asked for once");
    let repos = &resources.common.repos;
    assert!(
        repos
            .activity_streams
            .get_activity_streams(&tenant_id, user_id, "garmin", "993001")
            .await
            .unwrap()
            .is_none(),
        "an oversized file stores no samples"
    );
    assert!(
        repos
            .pushed_data
            .kept_file(&tenant_id, user_id, "garmin", "993001")
            .await
            .unwrap()
            .is_none(),
        "an oversized file is never kept for a retry"
    );
    let inbox = &resources.common.repos.garmin_push_inbox;
    assert!(
        inbox
            .claim_due_pushes(InboxClaim {
                now_ms: Utc::now().timestamp_millis() + 86_400_000,
                lease_ms: 60_000,
                max_attempts: 8,
                limit: 10,
            })
            .await
            .unwrap()
            .is_empty(),
        "the batch is settled, not handed back to ask Garmin again"
    );
}

/// Intraday `SpO2`, breathing-rate and stress readings have no reader, so
/// none is stored: the sleep and the day's Body Battery land, the heart-rate
/// series beside them still does, and a `pulseox` push is refused as a type
/// Dravr does not ingest.
#[tokio::test]
async fn intraday_spo2_breathing_and_stress_are_not_stored() {
    let resources = context(None).await;
    let (user_id, tenant_id) = seed_athlete(&resources).await;
    let date = (Utc::now() - ChronoDuration::days(1)).date_naive();
    let calendar = date.format("%Y-%m-%d").to_string();
    let day_start = date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();

    push(
        &resources,
        "dailies",
        json!([{
            "userId": GARMIN_USER,
            "calendarDate": calendar,
            "startTimeInSeconds": day_start,
            "restingHeartRateInBeatsPerMinute": 47,
            "timeOffsetHeartRateSamples": { "15": 58, "30": 60 },
        }]),
    )
    .await;
    push(
        &resources,
        "sleeps",
        json!([{
            "userId": GARMIN_USER,
            "calendarDate": calendar,
            "startTimeInSeconds": day_start - 7 * 3_600,
            "durationInSeconds": 25_200,
            "timeOffsetSleepSpo2": { "0": 95, "3600": 94 },
            "timeOffsetSleepRespiration": { "0": 14.0, "3600": 13.5 },
        }]),
    )
    .await;
    push(
        &resources,
        "stressDetails",
        json!([{
            "userId": GARMIN_USER,
            "calendarDate": calendar,
            "startTimeInSeconds": day_start,
            "timeOffsetStressLevelValues": { "0": 18, "180": 22 },
            "timeOffsetBodyBatteryValues": { "0": 61, "180": 64 },
        }]),
    )
    .await;

    let sources = resources
        .common
        .repos
        .data_sources
        .list_data_sources(user_id, &tenant_id)
        .await
        .unwrap();
    assert_eq!(sources.len(), 1, "the Garmin API's data source");
    let points = &resources.common.repos.time_series_points;
    let range = TimeRange::new(
        Utc::now() - ChronoDuration::days(3),
        Utc::now() + ChronoDuration::days(1),
    );
    let latest = |series: SeriesType| {
        let range = range.clone();
        let source = sources[0].id.clone();
        async move {
            !points
                .query(&source, series.id(), &range)
                .await
                .unwrap()
                .points
                .is_empty()
        }
    };
    assert!(
        latest(SeriesType::HeartRate).await,
        "heart rate has a reader"
    );
    for series in [
        SeriesType::Spo2,
        SeriesType::RespiratoryRate,
        SeriesType::StressLevel,
    ] {
        assert!(!latest(series).await, "bug: {series:?} has no reader");
    }
    assert_eq!(
        garmin_recovery(&resources, user_id, &tenant_id).await[0].readiness_score,
        Some(64),
        "the day's Body Battery still lands"
    );

    let pulseox = json!({ "pulseox": [{
        "userId": GARMIN_USER,
        "startTimeInSeconds": day_start,
        "timeOffsetSpo2Values": { "0": 96 },
    }]});
    assert_eq!(
        post_as(&resources, PATH_TOKEN, "pulseox", Some(CLIENT_ID), &pulseox).await,
        StatusCode::NOT_FOUND,
        "a pulseox push is a type Dravr does not ingest"
    );
}
