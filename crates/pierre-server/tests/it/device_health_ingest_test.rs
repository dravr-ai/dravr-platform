// ABOUTME: The phone's health store synced end to end — /api/device-health sync and delete, the reads that serve what landed
// ABOUTME: Real routes, barrage reader, live store and wellness projection over the test database; samples in the companion app's wire shape

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Owl phase 9 (carnet#1548). The companion app sends `HealthKit`'s raw
//! samples to `POST /api/device-health/apple_health/sync`; these tests send
//! them as it does and read back what the athlete, the agent and the stores
//! hold. Every sample is dated in the past few days at a fixed -04:00 offset.

use pierre_core::constants::oauth_providers::NoticeKind;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use dravr_riviere::{SeriesType, TimeRange};
use dravr_tronc::dates::{CivilDate, CivilDateTime, Elapsed, Timestamp};
use pierre_core::constants::oauth::providers::{sensitive_display_name, SENSITIVE_T3};
use pierre_core::constants::oauth_providers::{
    APPLE_HEALTH, APPLE_HEALTH_ARCHIVE, HEALTH_CONNECT, UPLOAD,
};
use pierre_core::models::{ActivityBuilder, CreateUserMcpTokenRequest, SportType};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_database::repositories::RecordWindow;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::device_health::{device_health_routes, MAX_SYNC_BODY_BYTES};
use pierre_mcp_server::routes::health_records::health_records_routes;
use pierre_services::live_activities::WriteOrigin;
use pierre_services::provider_notice::grant_consent;
use pierre_tool_runtime::protocol::{UniversalRequest, UniversalToolExecutor};
use secrecy::ExposeSecret;
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::archive_import_test::{agent_activities, seed_athlete, Athlete};
use crate::common;
use crate::helpers::impersonation::{assert_support_refused, impersonation_of};

const SYNC: &str = "/api/device-health/apple_health/sync";
const DATA: &str = "/api/device-health/apple_health/data";
/// -04:00, the athlete's clock.
const EDT: i32 = -4 * 3600;
const SLEEP: &str = "HKCategoryTypeIdentifierSleepAnalysis";
const RESTING_HR: &str = "HKQuantityTypeIdentifierRestingHeartRate";
const GLUCOSE: &str = "HKQuantityTypeIdentifierBloodGlucose";
/// `HKWorkoutActivityTypeRunning`, `HKWorkoutActivityTypeCycling`.
const RUNNING: u32 = 37;
const CYCLING: u32 = 13;

/// The local date `days_ago` days before today.
fn day(days_ago: i64) -> CivilDate {
    (Timestamp::now() + Elapsed::seconds(i64::from(EDT))).utc_date() - Elapsed::days(days_ago)
}

/// `hour:minute` local on the date `days_ago` days before today, as UTC.
fn local(days_ago: i64, hour: u32, minute: u32) -> Timestamp {
    day(days_ago)
        .and_hms(hour, minute, 0)
        .and_then(CivilDateTime::and_utc)
        .expect("a valid time")
        - Elapsed::seconds(i64::from(EDT))
}

fn recorder(name: &str) -> Value {
    json!({ "name": name, "device_model": null })
}

/// A request from the iPhone's store carrying `parts`.
fn samples(parts: Value) -> Value {
    let mut body = json!({ "source": "apple_health" });
    if let (Some(body), Value::Object(parts)) = (body.as_object_mut(), parts) {
        body.extend(parts);
    }
    body
}

/// A workout of `activity_type` recorded by `by`, starting `days_ago` days
/// back at 06:15 local for 45 minutes.
fn workout(id: &str, activity_type: u32, by: &str, days_ago: i64) -> Value {
    json!({
        "id": id,
        "activity_type": activity_type,
        "start": local(days_ago, 6, 15),
        "end": local(days_ago, 7, 0),
        "utc_offset_seconds": EDT,
        "duration_seconds": 2_700.0,
        "distance": { "value": 9.0, "unit": "km" },
        "energy": { "value": 450.0, "unit": "kcal" },
        "recorder": recorder(by),
    })
}

/// One sleep stage sample (`HKCategoryValueSleepAnalysis` raw value).
fn stage(id: &str, from: Timestamp, to: Timestamp, value: i64) -> Value {
    json!({
        "id": id, "type": SLEEP, "value": value, "start": from, "end": to,
        "utc_offset_seconds": EDT, "recorder": recorder("Watch"),
    })
}

/// A night from 23:30 the evening `days_ago + 1` days back to 06:30: core
/// then deep then REM.
fn night(prefix: &str, days_ago: i64) -> Vec<Value> {
    let evening = local(days_ago + 1, 23, 30);
    vec![
        stage(
            &format!("{prefix}-1"),
            evening,
            evening + Elapsed::hours(3),
            3,
        ),
        stage(
            &format!("{prefix}-2"),
            evening + Elapsed::hours(3),
            evening + Elapsed::hours(5),
            4,
        ),
        stage(
            &format!("{prefix}-3"),
            evening + Elapsed::hours(5),
            evening + Elapsed::hours(7),
            5,
        ),
    ]
}

/// The sleep day the night ending `days_ago` days back falls in: local noon
/// the day before to local noon that day.
fn sleep_day(days_ago: i64) -> Value {
    json!({ "from": local(days_ago + 1, 12, 0), "to": local(days_ago, 12, 0) })
}

fn quantity(id: &str, kind: &str, value: f64, unit: &str, at: Timestamp) -> Value {
    json!({
        "id": id, "type": kind, "value": value, "unit": unit, "start": at, "end": at,
        "utc_offset_seconds": EDT, "recorder": recorder("Watch"),
    })
}

/// A whole local day of one quantity type, re-read.
fn day_window(kind: &str, days_ago: i64) -> Value {
    json!({
        "type": kind, "from": local(days_ago, 0, 0), "to": local(days_ago - 1, 0, 0),
        "utc_offset_seconds": EDT,
    })
}

fn steps(days_ago: i64, count: u32) -> Value {
    json!({
        "date": day(days_ago), "utc_offset_seconds": EDT, "steps": count,
        "active_kcal": 480.0, "recorder": recorder("Health"),
    })
}

/// A daily mood (`HKStateOfMindKind` 2), pleasant (classification 6).
fn mood(id: &str, days_ago: i64) -> Value {
    json!({
        "id": id, "kind": 2, "valence": 0.6, "classification": 6,
        "start": local(days_ago, 20, 0), "utc_offset_seconds": EDT,
        "recorder": recorder("Health"),
    })
}

/// A request to `router` with `authorization` as is.
async fn send(
    router: axum::Router,
    method: &str,
    uri: &str,
    authorization: &str,
    body: Option<&Value>,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", authorization)
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
        .expect("a request");
    let response = router.oneshot(request).await.expect("the router answers");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a body");
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
    )
}

/// A device-health route on the athlete's own session.
async fn call(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    method: &str,
    uri: &str,
    body: Option<&Value>,
) -> (StatusCode, Value) {
    send(
        device_health_routes().with_state(Arc::clone(resources)),
        method,
        uri,
        &format!("Bearer {}", athlete.token),
        body,
    )
    .await
}

/// Sync `body` and expect it read whole, nothing refused.
async fn sync(resources: &Arc<ServerContext>, athlete: &Athlete, body: &Value) -> Value {
    let (status, counts) = call(resources, athlete, "POST", SYNC, Some(body)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: the sync was refused: {counts}"
    );
    assert_eq!(counts["refused"], 0, "bug: a sample was refused: {counts}");
    counts
}

/// Every night the athlete holds in the past two weeks, oldest first, as
/// `(start, end)`.
async fn nights(resources: &ServerContext, athlete: &Athlete) -> Vec<(Timestamp, Timestamp)> {
    let mut held: Vec<_> = resources
        .common
        .repos
        .sleep
        .get_sleep_sessions(
            athlete.user_id,
            &athlete.tenant,
            Timestamp::now() - Elapsed::days(14),
            Timestamp::now(),
        )
        .await
        .expect("nights")
        .into_iter()
        .map(|night| (night.start_datetime, night.end_datetime))
        .collect();
    held.sort();
    held
}

/// The athlete's self-reports, as `(kind, value code)`.
async fn self_reports(resources: &ServerContext, athlete: &Athlete) -> Vec<(String, String)> {
    resources
        .common
        .repos
        .health_records
        .list_self_reports(
            &athlete.tenant,
            athlete.user_id,
            RecordWindow {
                since: None,
                until: None,
                limit: 200,
            },
        )
        .await
        .expect("self-reports")
        .into_iter()
        .map(|report| (report.kind, report.value_code.unwrap_or_default()))
        .collect()
}

/// The athlete's recovery days' resting heart rates, by date.
async fn resting_rates(resources: &ServerContext, athlete: &Athlete) -> Vec<(CivilDate, f64)> {
    let mut rows: Vec<_> = resources
        .common
        .repos
        .recovery
        .get_recovery_metrics(
            athlete.user_id,
            &athlete.tenant,
            Timestamp::now() - Elapsed::days(14),
            Timestamp::now(),
        )
        .await
        .expect("recovery days")
        .into_iter()
        .filter_map(|row| {
            row.resting_heart_rate
                .map(|rate| (row.date, f64::from(rate)))
        })
        .collect();
    rows.sort_by_key(|(date, _)| *date);
    rows
}

/// The athlete's glucose readings under the store's key, oldest first.
async fn glucose(resources: &ServerContext, athlete: &Athlete) -> Vec<f64> {
    let repos = &resources.common.repos;
    let sources = repos
        .data_sources
        .list_data_sources(athlete.user_id, &athlete.tenant)
        .await
        .expect("data sources");
    let mut readings = Vec::new();
    for source in sources
        .iter()
        .filter(|source| source.provider == APPLE_HEALTH)
    {
        let found = repos
            .time_series_points
            .query(
                &source.id,
                SeriesType::BloodGlucose.id(),
                &TimeRange::new(Timestamp::now() - Elapsed::days(14), Timestamp::now()),
            )
            .await
            .expect("glucose");
        readings.extend(found.points.iter().map(|point| point.value));
    }
    readings
}

/// The athlete's cached activities under the store's key.
async fn cached(resources: &ServerContext, athlete: &Athlete) -> usize {
    resources
        .common
        .repos
        .activity_cache
        .get_cached_activities(
            athlete.user_id,
            &athlete.tenant,
            Some(APPLE_HEALTH),
            Timestamp::now() - Elapsed::days(30),
            Timestamp::now(),
            100,
        )
        .await
        .expect("cached activities")
        .len()
}

fn activity_count(payload: &Value) -> usize {
    payload["activities"].as_array().map_or(0, Vec::len)
}

async fn athlete_world(label: &str) -> (Arc<ServerContext>, Athlete) {
    let resources = common::create_test_server_resources()
        .await
        .expect("server resources");
    let athlete = seed_athlete(&resources, label).await;
    (resources, athlete)
}

#[tokio::test]
async fn a_workout_lands_under_apple_health_and_a_garmin_recording_of_it_reads_as_one() {
    let (resources, athlete) = athlete_world("dh-workout").await;
    let counts = sync(
        &resources,
        &athlete,
        &samples(json!({ "workouts": [workout("AAAA0000-0000-4000-8000-000000000001", RUNNING, "Connect", 2)] })),
    )
    .await;
    assert_eq!(
        counts["activities"], 1,
        "bug: the run was not held: {counts}"
    );
    assert_eq!(
        cached(&resources, &athlete).await,
        1,
        "bug: the run is not in the cache"
    );

    // The same run uploaded from the watch's own file.
    let watch = ActivityBuilder::new(
        "fit-run",
        "Morning Run",
        SportType::Run,
        local(2, 6, 15),
        2_700,
        UPLOAD,
    )
    .device_name("Forerunner 965".to_owned())
    .distance_meters(9_000.0)
    .build();
    resources
        .common
        .live_activities
        .ingest(
            athlete.user_id,
            athlete.tenant,
            UPLOAD,
            &[watch],
            Timestamp::now(),
            WriteOrigin::Import,
        )
        .await
        .expect("the upload lands");

    let read = agent_activities(&resources, &athlete, json!({ "limit": 20 })).await;
    assert_eq!(
        activity_count(&read),
        1,
        "bug: the iPhone's copy and the watch's file of one run read as two: {read}"
    );
}

#[tokio::test]
async fn two_recorders_copies_of_one_workout_with_two_sports_read_as_one() {
    let (resources, athlete) = athlete_world("dh-recorders").await;
    sync(
        &resources,
        &athlete,
        &samples(json!({ "workouts": [
            workout("BBBB0000-0000-4000-8000-000000000001", RUNNING, "Strava", 2),
            workout("BBBB0000-0000-4000-8000-000000000002", CYCLING, "Connect", 2),
        ] })),
    )
    .await;
    assert_eq!(
        cached(&resources, &athlete).await,
        2,
        "bug: both copies are stored"
    );
    let read = agent_activities(&resources, &athlete, json!({ "limit": 20 })).await;
    assert_eq!(
        activity_count(&read),
        1,
        "bug: Strava's and Garmin Connect's copies of one session read as two: {read}"
    );
    let pinned = agent_activities(&resources, &athlete, json!({ "provider": APPLE_HEALTH })).await;
    assert_eq!(
        activity_count(&pinned),
        1,
        "bug: a read pinned to apple_health does not serve the phone's rows: {pinned}"
    );
}

/// A read pinned to `apple_health` serves what the iPhone synced and the
/// Apple Health export the athlete imported beside it, never the export in
/// its place.
#[tokio::test]
async fn a_read_pinned_to_apple_health_serves_the_phone_and_its_export() {
    let (resources, athlete) = athlete_world("dh-pinned").await;
    sync(
        &resources,
        &athlete,
        &samples(json!({ "workouts": [workout("CAFE0000-0000-4000-8000-000000000001", RUNNING, "Strava", 2)] })),
    )
    .await;
    let exported = ActivityBuilder::new(
        "export-walk",
        "Walk",
        SportType::Walk,
        local(6, 17, 0),
        1_800,
        APPLE_HEALTH_ARCHIVE,
    )
    .build();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(
            athlete.user_id,
            &athlete.tenant,
            APPLE_HEALTH_ARCHIVE,
            &[exported],
        )
        .await
        .expect("the export's row");

    let pinned = agent_activities(
        &resources,
        &athlete,
        json!({ "provider": APPLE_HEALTH, "limit": 20 }),
    )
    .await;
    assert_eq!(
        activity_count(&pinned),
        2,
        "bug: a read pinned to apple_health does not serve both the phone's and the export's rows: {pinned}"
    );
}

#[tokio::test]
async fn a_night_is_replaced_by_its_window_and_deleted_when_the_window_is_emptied() {
    let (resources, athlete) = athlete_world("dh-night").await;
    let counts = sync(
        &resources,
        &athlete,
        &samples(json!({ "categories": night("n1", 2), "sleep_windows": [sleep_day(2)] })),
    )
    .await;
    assert_eq!(counts["nights"], 1, "bug: one night written: {counts}");
    let first = nights(&resources, &athlete).await;
    assert_eq!(first.len(), 1, "bug: the night is held: {first:?}");

    // The phone re-reads the window with the night's last stage gone.
    let mut shorter = night("n1", 2);
    shorter.pop();
    sync(
        &resources,
        &athlete,
        &samples(json!({ "categories": shorter, "sleep_windows": [sleep_day(2)] })),
    )
    .await;
    let replaced = nights(&resources, &athlete).await;
    assert_eq!(
        replaced.len(),
        1,
        "bug: the re-read night was added beside the old one: {replaced:?}"
    );
    assert!(
        replaced[0].1 < first[0].1,
        "bug: the night still ends where its deleted stage did: {replaced:?}"
    );

    // Every sample of the window deleted: an empty window.
    sync(
        &resources,
        &athlete,
        &samples(json!({ "sleep_windows": [sleep_day(2)] })),
    )
    .await;
    assert!(
        nights(&resources, &athlete).await.is_empty(),
        "bug: a night the store no longer holds survived its emptied window"
    );
}

#[tokio::test]
async fn a_night_crossing_local_noon_survives_the_next_windows_resend() {
    let (resources, athlete) = athlete_world("dh-noon").await;
    // A late sleep: 04:00 to 13:00 local, crossing the sleep day's noon, so
    // the phone widens the window to its end.
    let late = vec![stage("late-1", local(3, 4, 0), local(3, 13, 0), 3)];
    let widened = json!({ "from": local(4, 12, 0), "to": local(3, 13, 0) });
    sync(
        &resources,
        &athlete,
        &samples(json!({ "categories": late, "sleep_windows": [widened] })),
    )
    .await;
    assert_eq!(
        nights(&resources, &athlete).await.len(),
        1,
        "bug: the late night is held"
    );

    // The next sleep day changes: the phone widens its window back to every
    // sample overlapping it, the late night's included, and sends them all.
    let mut next = vec![stage("late-1", local(3, 4, 0), local(3, 13, 0), 3)];
    next.extend(night("n2", 2));
    let back = json!({ "from": local(3, 4, 0), "to": local(2, 12, 0) });
    sync(
        &resources,
        &athlete,
        &samples(json!({ "categories": next, "sleep_windows": [back] })),
    )
    .await;
    let held = nights(&resources, &athlete).await;
    assert_eq!(
        held.len(),
        2,
        "bug: the night crossing noon was cut or lost: {held:?}"
    );
    assert_eq!(
        held[0],
        (local(3, 4, 0), local(3, 13, 0)),
        "bug: the night crossing noon is not whole: {held:?}"
    );
}

#[tokio::test]
async fn a_days_steps_land_as_the_whole_day_and_a_resync_replaces_them() {
    let (resources, athlete) = athlete_world("dh-steps").await;
    let counts = sync(
        &resources,
        &athlete,
        &samples(json!({ "days": [steps(1, 8_000)] })),
    )
    .await;
    assert_eq!(counts["days"], 1, "bug: one day written: {counts}");
    sync(
        &resources,
        &athlete,
        &samples(json!({ "days": [steps(1, 9_100)] })),
    )
    .await;
    let rows = resources
        .common
        .repos
        .daily_activity
        .get_daily_activity(
            athlete.user_id,
            &athlete.tenant,
            Timestamp::now() - Elapsed::days(7),
            Timestamp::now(),
        )
        .await
        .expect("daily activity");
    let held: Vec<(CivilDate, Option<u32>)> =
        rows.iter().map(|row| (row.date, row.steps)).collect();
    assert_eq!(
        held,
        [(day(1), Some(9_100))],
        "bug: the day's second total did not replace the first: {held:?}"
    );
    let sources = resources
        .common
        .repos
        .data_sources
        .list_data_sources(athlete.user_id, &athlete.tenant)
        .await
        .expect("data sources");
    assert_eq!(
        sources
            .iter()
            .filter(|source| source.provider == APPLE_HEALTH)
            .count(),
        1,
        "bug: two syncs recorded two data sources: {sources:?}"
    );
}

#[tokio::test]
async fn a_deleted_resting_heart_rate_day_resent_empty_leaves_no_recovery_row() {
    let (resources, athlete) = athlete_world("dh-resting").await;
    let rate = quantity(
        "CCCC0000-0000-4000-8000-000000000001",
        RESTING_HR,
        48.0,
        "count/min",
        local(2, 7, 0),
    );
    let counts = sync(
        &resources,
        &athlete,
        &samples(json!({ "quantities": [rate] })),
    )
    .await;
    assert_eq!(
        counts["days"], 1,
        "bug: the recovery day was written: {counts}"
    );
    assert_eq!(
        resting_rates(&resources, &athlete).await,
        [(day(2), 48.0)],
        "bug: the day's resting heart rate is held"
    );

    sync(
        &resources,
        &athlete,
        &samples(json!({ "quantity_windows": [day_window(RESTING_HR, 2)] })),
    )
    .await;
    assert!(
        resting_rates(&resources, &athlete).await.is_empty(),
        "bug: a deleted reading's day survived its emptied window"
    );
}

#[tokio::test]
async fn a_glucose_window_removes_the_deleted_reading_and_keeps_the_resent_one() {
    let (resources, athlete) = athlete_world("dh-glucose").await;
    let morning = quantity(
        "DDDD0000-0000-4000-8000-000000000001",
        GLUCOSE,
        95.0,
        "mg/dL",
        local(2, 8, 0),
    );
    let evening = quantity(
        "DDDD0000-0000-4000-8000-000000000002",
        GLUCOSE,
        110.0,
        "mg/dL",
        local(2, 19, 0),
    );
    sync(
        &resources,
        &athlete,
        &samples(json!({ "quantities": [morning, evening.clone()] })),
    )
    .await;
    assert_eq!(
        glucose(&resources, &athlete).await,
        [95.0, 110.0],
        "bug: both readings are held"
    );

    // The morning reading was deleted: its day re-read holds the evening one.
    let window = json!({ "type": GLUCOSE, "from": local(2, 0, 0), "to": local(1, 0, 0) });
    sync(
        &resources,
        &athlete,
        &samples(json!({ "quantities": [evening], "quantity_windows": [window] })),
    )
    .await;
    assert_eq!(
        glucose(&resources, &athlete).await,
        [110.0],
        "bug: the deleted reading survived, or the re-sent one went with it"
    );
}

#[tokio::test]
async fn a_mood_is_the_athletes_and_reaches_the_agent_only_under_sensitive_t3() {
    let (resources, athlete) = athlete_world("dh-mood").await;
    let counts = sync(
        &resources,
        &athlete,
        &samples(json!({ "states_of_mind": [mood("EEEE0000-0000-4000-8000-000000000001", 1)] })),
    )
    .await;
    assert_eq!(
        counts["self_reports"], 1,
        "bug: the mood was not written: {counts}"
    );
    assert_eq!(
        self_reports(&resources, &athlete).await,
        [("mood".to_owned(), "pleasant".to_owned())],
        "bug: the mood is stored as a mood, its classification its code"
    );

    // The athlete's own health-records screen serves it.
    let (status, screen) = send(
        health_records_routes().with_state(Arc::clone(&resources)),
        "GET",
        "/api/me/health-records?kind=self_reports",
        &format!("Bearer {}", athlete.token),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: the screen's read failed: {screen}"
    );
    assert!(
        screen.to_string().contains("\"mood\""),
        "bug: the screen does not serve the mood: {screen}"
    );

    let agent = || async {
        let response = UniversalToolExecutor::new(resources.clone())
            .with_scopes(OAuthScope::self_grant())
            .with_transport(Transport::WebApp)
            .execute_tool(UniversalRequest {
                tool_name: "get_health_records".to_owned(),
                parameters: json!({ "kind": "self_reports" }),
                user_id: athlete.user_id.to_string(),
                protocol: "mcp".to_owned(),
                tenant_id: Some(athlete.tenant.to_string()),
            })
            .await
            .expect("the tool dispatches");
        response.result.unwrap_or(Value::Null)
    };
    let withheld = agent().await;
    assert!(
        !withheld.to_string().contains("\"mood\""),
        "bug: a T3 mood reached the agent without sensitive_t3: {withheld}"
    );
    grant_consent(
        &resources.common.repos,
        athlete.tenant.as_uuid(),
        athlete.user_id,
        SENSITIVE_T3,
        NoticeKind::AiConsent,
        sensitive_display_name(SENSITIVE_T3).expect("a consent class"),
    )
    .await
    .expect("consent given");
    let allowed = agent().await;
    assert!(
        allowed.to_string().contains("\"mood\""),
        "bug: the agent does not read the mood once sensitive_t3 is given: {allowed}"
    );
}

#[tokio::test]
async fn a_deleted_workout_is_erased_for_good_and_a_deleted_mood_is_deleted() {
    let (resources, athlete) = athlete_world("dh-deletes").await;
    let run_id = "FFFF0000-0000-4000-8000-000000000001";
    let mood_id = "FFFF0000-0000-4000-8000-000000000002";
    sync(
        &resources,
        &athlete,
        &samples(json!({
            "workouts": [workout(run_id, RUNNING, "Strava", 2)],
            "states_of_mind": [mood(mood_id, 1)],
        })),
    )
    .await;
    let counts = sync(
        &resources,
        &athlete,
        &samples(json!({ "deleted": { "workouts": [run_id], "self_reports": [mood_id] } })),
    )
    .await;
    assert_eq!(
        counts["erased"], 2,
        "bug: the workout and the mood were not both removed: {counts}"
    );
    assert_eq!(
        cached(&resources, &athlete).await,
        0,
        "bug: the deleted workout is still served"
    );
    assert!(
        self_reports(&resources, &athlete).await.is_empty(),
        "bug: the deleted mood is still held"
    );

    // A late re-send of the workout never brings it back.
    let (status, again) = call(
        &resources,
        &athlete,
        "POST",
        SYNC,
        Some(&samples(
            json!({ "workouts": [workout(run_id, RUNNING, "Strava", 2)] }),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(
        again["refused"], 1,
        "bug: the erased workout was not refused: {again}"
    );
    assert_eq!(
        cached(&resources, &athlete).await,
        0,
        "bug: a re-send brought the erased workout back"
    );
}

/// Everything one sync can hold under the key, for the athlete.
fn everything() -> Value {
    samples(json!({
        "workouts": [workout("A1000000-0000-4000-8000-000000000001", RUNNING, "Strava", 2)],
        "categories": night("all", 2),
        "sleep_windows": [sleep_day(2)],
        "quantities": [
            quantity("A1000000-0000-4000-8000-000000000002", RESTING_HR, 50.0, "count/min", local(2, 7, 0)),
            quantity("A1000000-0000-4000-8000-000000000003", GLUCOSE, 101.0, "mg/dL", local(2, 8, 0)),
        ],
        "days": [steps(2, 7_500)],
        "states_of_mind": [mood("A1000000-0000-4000-8000-000000000004", 2)],
    }))
}

/// Whether the athlete holds anything under the key.
async fn holds_any(resources: &ServerContext, athlete: &Athlete) -> bool {
    cached(resources, athlete).await > 0
        || !nights(resources, athlete).await.is_empty()
        || !resting_rates(resources, athlete).await.is_empty()
        || !self_reports(resources, athlete).await.is_empty()
        || !glucose(resources, athlete).await.is_empty()
}

#[tokio::test]
async fn deleting_the_synced_data_leaves_no_row_under_the_key() {
    let (resources, athlete) = athlete_world("dh-purge").await;
    sync(&resources, &athlete, &everything()).await;
    assert!(
        holds_any(&resources, &athlete).await,
        "bug: the sync held nothing"
    );

    let (status, removed) = call(&resources, &athlete, "DELETE", DATA, None).await;
    assert_eq!(status, StatusCode::OK, "bug: the delete failed: {removed}");
    assert_eq!(
        removed["activities"], 1,
        "bug: the activity count: {removed}"
    );
    assert_eq!(removed["nights"], 1, "bug: the night count: {removed}");
    assert_eq!(
        removed["self_reports"], 1,
        "bug: the self-report count: {removed}"
    );
    assert!(
        removed["days"].as_u64().is_some_and(|days| days >= 2),
        "bug: the recovery and activity days were not counted: {removed}"
    );
    assert!(
        !holds_any(&resources, &athlete).await,
        "bug: a row survived the delete of the synced data"
    );
}

#[tokio::test]
async fn an_api_key_an_mcp_token_and_a_support_session_are_refused() {
    let (resources, athlete) = athlete_world("dh-refused").await;
    let repos = resources.agent.database.repositories();
    let api_key = common::issue_test_api_key(&resources.agent.database, athlete.user_id, "dh key")
        .await
        .expect("an API key");
    let mcp_token = repos
        .user_mcp_tokens
        .create_token(
            athlete.user_id,
            &CreateUserMcpTokenRequest {
                name: "Claude Desktop".to_owned(),
                expires_in_days: None,
            },
        )
        .await
        .expect("an MCP token")
        .token_value;
    let support = impersonation_of(&resources, &format!("Bearer {}", athlete.token)).await;
    let delegated = common::delegated_test_token_for_tenant(
        &resources,
        athlete.user_id,
        Some(athlete.tenant.to_string()),
    );
    let body = everything();

    for (credential, who) in [
        (api_key.clone(), "an API key"),
        (
            format!("Bearer {}", mcp_token.expose_secret()),
            "a personal MCP token",
        ),
        (format!("Bearer {delegated}"), "a delegated OAuth grant"),
    ] {
        for (method, uri) in [("POST", SYNC), ("DELETE", DATA)] {
            let (status, answer) = send(
                device_health_routes().with_state(Arc::clone(&resources)),
                method,
                uri,
                &credential,
                Some(&body),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::FORBIDDEN,
                "bug: {who} was not refused at {method} {uri}: {answer}"
            );
        }
    }
    for (method, uri) in [("POST", SYNC), ("DELETE", DATA)] {
        let (status, answer) = send(
            device_health_routes().with_state(Arc::clone(&resources)),
            method,
            uri,
            &support,
            Some(&body),
        )
        .await;
        assert_support_refused(
            status.as_u16(),
            &answer.to_string(),
            "sync the athlete's phone",
        );
    }
    assert!(
        !holds_any(&resources, &athlete).await,
        "bug: a refused credential wrote under the key"
    );
}

#[tokio::test]
async fn an_unknown_store_is_not_found_and_an_unreadable_body_is_a_bad_request() {
    let (resources, athlete) = athlete_world("dh-input").await;
    let (status, _) = call(
        &resources,
        &athlete,
        "POST",
        "/api/device-health/fitbit/sync",
        Some(&samples(json!({}))),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "bug: a store not offered was served"
    );
    let (status, answer) = call(
        &resources,
        &athlete,
        "POST",
        SYNC,
        Some(&json!({ "source": "apple_health", "surprise": [] })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "bug: an unknown field was read: {answer}"
    );
    let past_cap: Vec<Value> = (0..2_001)
        .map(|n| json!({ "date": day(1), "utc_offset_seconds": EDT, "steps": n, "recorder": recorder("Health") }))
        .collect();
    let (status, answer) = call(
        &resources,
        &athlete,
        "POST",
        SYNC,
        Some(&samples(json!({ "days": past_cap }))),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "bug: a request past the sample cap was read: {answer}"
    );

    // A body past the 4 MiB cap is refused before it is read.
    let oversized = samples(json!({ "padding": "x".repeat(MAX_SYNC_BODY_BYTES) }));
    let (status, _) = call(&resources, &athlete, "POST", SYNC, Some(&oversized)).await;
    assert_eq!(
        status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "bug: a body past the cap was read"
    );
}

#[tokio::test]
async fn a_health_connect_workout_and_night_land_under_health_connect() {
    let (resources, athlete) = athlete_world("dh-hc").await;
    // `ExerciseSessionRecord.EXERCISE_TYPE_RUNNING` is 56; sleep stage 4 is
    // light, 5 deep.
    let run = json!({
        "id": "hc-session-1", "activity_type": 56,
        "start": local(2, 6, 15), "end": local(2, 7, 0), "utc_offset_seconds": EDT,
        "distance": { "value": 9_000.0, "unit": "m" },
        "recorder": recorder("com.strava"),
    });
    let stage = |n: u32, from: Timestamp, hours: i64, value: i64| {
        json!({
            "id": format!("hc-sleep-1-{n}"), "type": "SleepSessionRecord", "value": value,
            "start": from, "end": from + Elapsed::hours(hours), "utc_offset_seconds": EDT,
            "recorder": recorder("com.ouraring.oura"),
        })
    };
    let evening = local(3, 23, 30);
    let body = json!({
        "source": "health_connect",
        "workouts": [run],
        "categories": [stage(0, evening, 3, 4), stage(1, evening + Elapsed::hours(3), 2, 5)],
        "sleep_windows": [sleep_day(2)],
    });
    let (status, counts) = call(
        &resources,
        &athlete,
        "POST",
        "/api/device-health/health_connect/sync",
        Some(&body),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: the Health Connect sync failed: {counts}"
    );
    assert_eq!(
        counts["refused"], 0,
        "bug: a Health Connect sample was refused: {counts}"
    );
    assert_eq!(
        (counts["activities"].as_u64(), counts["nights"].as_u64()),
        (Some(1), Some(1)),
        "bug: the run and the night did not land: {counts}"
    );
    let held = resources
        .common
        .repos
        .activity_cache
        .get_cached_activities(
            athlete.user_id,
            &athlete.tenant,
            Some(HEALTH_CONNECT),
            Timestamp::now() - Elapsed::days(30),
            Timestamp::now(),
            10,
        )
        .await
        .expect("cached activities");
    assert_eq!(
        held.len(),
        1,
        "bug: the run is not cached under health_connect"
    );
    assert_eq!(
        cached(&resources, &athlete).await,
        0,
        "bug: a Health Connect run landed under apple_health"
    );
}

#[tokio::test]
async fn one_athletes_sync_and_delete_never_touch_another_athletes_rows() {
    let (resources, athlete) = athlete_world("dh-a").await;
    let other = seed_athlete(&resources, "dh-b").await;
    assert_ne!(
        athlete.tenant, other.tenant,
        "the two athletes act in two tenants"
    );
    // Both hold the same samples, ids included.
    sync(&resources, &athlete, &everything()).await;
    sync(&resources, &other, &everything()).await;
    assert!(
        holds_any(&resources, &other).await,
        "bug: the other athlete's sync held nothing"
    );
    let before = (
        nights(&resources, &other).await,
        resting_rates(&resources, &other).await,
        self_reports(&resources, &other).await,
        glucose(&resources, &other).await,
        cached(&resources, &other).await,
    );

    // The first athlete empties every window and deletes every sample, then
    // deletes the store's data.
    sync(
        &resources,
        &athlete,
        &samples(json!({
            "sleep_windows": [sleep_day(2)],
            "quantity_windows": [
                day_window(RESTING_HR, 2),
                { "type": GLUCOSE, "from": local(2, 0, 0), "to": local(1, 0, 0) },
            ],
            "deleted": {
                "workouts": ["A1000000-0000-4000-8000-000000000001"],
                "self_reports": ["A1000000-0000-4000-8000-000000000004"],
            },
        })),
    )
    .await;
    let (status, removed) = call(&resources, &athlete, "DELETE", DATA, None).await;
    assert_eq!(status, StatusCode::OK, "{removed}");
    assert!(
        !holds_any(&resources, &athlete).await,
        "bug: the first athlete kept a row"
    );

    let after = (
        nights(&resources, &other).await,
        resting_rates(&resources, &other).await,
        self_reports(&resources, &other).await,
        glucose(&resources, &other).await,
        cached(&resources, &other).await,
    );
    assert_eq!(
        after, before,
        "bug: one athlete's deletes reached another's rows"
    );
}
