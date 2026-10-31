// ABOUTME: A Strava workout held live (API or usr/pwd capture) and in the imported Strava export is one session
// ABOUTME: Lists and the training-load rollup count it once; a by-id read of it returns the copy standing for it, samples included

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1242: an athlete connected to Strava who also imports their Strava
//! export holds each workout twice in the activity cache, under the live
//! provider and under `strava_archive`. Both copies carry Strava's activity
//! id, so the session merger joins them on it — not only on their times,
//! which a date-only capture (the usr/pwd list read, at midnight UTC of the
//! athlete's day) and an evening export start (past midnight UTC) never
//! share, nor on their sport, which the athlete may have relabelled on
//! Strava after the device recorded it.
//!
//! The live copy here is the usr/pwd capture, served by a stand-in scraper
//! through the production fetch path; the export is imported through the
//! production import, on barrage's memory stores.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
#![cfg(feature = "tools-data")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::constants::oauth_providers::{
    GARMIN, GARMIN_ARCHIVE, SCIOTTE, SCIOTTE_GARMIN, STRAVA, STRAVA_ARCHIVE, UPLOAD,
};
use pierre_core::models::{
    Activity, ActivityBuilder, ConnectionType, DailyTrainingState, SportType, ThresholdProvenance,
    TimeSeriesData, UserOAuthToken, UserPhysiologicalProfile,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_database::repositories::{ActivityStreamsWrite, StreamOrigin};
use pierre_fitness_compute::training_history_compute::CTL_WINDOW_DAYS;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::protocol::provider_helpers::{
    by_id_provider, by_id_reader, fetch_activity_from_provider,
};
use pierre_tool_runtime::protocol::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::training_history_compute::{
    compute_and_persist_history, fetch_history_rows,
};
use serde_json::{json, Value};

use crate::archive_import_test::{
    agent_activities, import_export, imports_serving, one_ride_export, seed_athlete, Athlete,
};
use crate::common::{
    create_test_server_resources_with_archive_imports,
    create_test_server_resources_with_archive_imports_and_config, test_server_config_with_sciotte,
};
use crate::helpers::sciotte_mock::{seed_sciotte_session, spawn_mock_scraper_with_activity};

/// Strava's id for the workout both copies hold.
const RIDE_ID: &str = "16012345678";

/// Length of the ride, in seconds.
const RIDE_SECONDS: u32 = 2_700;

/// Days of history behind today: past the 72-day warm-up of the whole
/// 42-day window.
const DEEP_DAYS: i64 = 130;

/// How many days ago the athlete rode, on their own calendar.
const RIDE_DAYS_AGO: i64 = 4;

/// The athlete's day of the ride, at midnight UTC: what a date-only capture
/// records as its start.
fn ride_day() -> DateTime<Utc> {
    let day = Utc::now().date_naive() - Duration::days(RIDE_DAYS_AGO);
    Utc.from_utc_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
}

/// The ride's real start: 21:30 in Montreal on [`ride_day`], 01:30 UTC the
/// next day, as the export records it.
fn evening_start() -> DateTime<Utc> {
    ride_day() + Duration::hours(25) + Duration::minutes(30)
}

/// The capture's JSON as the scraper serves it: date-only, and the sport the
/// athlete relabelled the ride to on Strava.
fn date_only_capture_json() -> Value {
    json!({
        "id": RIDE_ID,
        "name": "Sortie du soir",
        "sport_type": "mountain_bike",
        "start_date": ride_day().to_rfc3339(),
        "duration_seconds": RIDE_SECONDS,
        "provider": "strava",
        "distance_meters": 21_600.0,
    })
}

/// The ids of the activities a `get_activities` payload lists, in order.
fn listed_ids(payload: &Value) -> Vec<String> {
    payload["activities"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row["id"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Connect `athlete` to Strava through the usr/pwd capture.
async fn connect_capture(resources: &Arc<ServerContext>, athlete: &Athlete) {
    resources
        .common
        .repos
        .provider_connections
        .register_connection(
            athlete.user_id,
            athlete.tenant,
            SCIOTTE,
            &ConnectionType::Manual,
            None,
        )
        .await
        .unwrap();
    seed_sciotte_session(resources, athlete.user_id, athlete.tenant).await;
}

/// Asks the agent's activity list, unpinned and pinned to Strava, and
/// asserts each lists the ride once.
async fn assert_listed_once(resources: &Arc<ServerContext>, athlete: &Athlete) {
    for parameters in [
        json!({ "limit": 10 }),
        json!({ "limit": 10, "provider": STRAVA }),
    ] {
        let payload = agent_activities(resources, athlete, parameters.clone()).await;
        let ids = listed_ids(&payload);
        assert_eq!(
            ids.iter().filter(|id| *id == RIDE_ID).count(),
            1,
            "{parameters}: the ride is listed once, got {ids:?}: {payload}"
        );
    }
}

#[tokio::test]
async fn a_date_only_capture_and_the_export_of_one_ride_are_listed_once() {
    let scraper = spawn_mock_scraper_with_activity(date_only_capture_json()).await;
    let (imports, _) = imports_serving(one_ride_export(RIDE_ID, evening_start(), RIDE_SECONDS));
    let resources = create_test_server_resources_with_archive_imports_and_config(
        imports,
        Some(test_server_config_with_sciotte(&scraper)),
    )
    .await
    .unwrap();
    let athlete = seed_athlete(&resources, "strava-twin-list").await;
    connect_capture(&resources, &athlete).await;
    let finished = import_export(&resources, &athlete).await;
    assert_eq!(finished["imported"], 1, "{finished}");

    assert_listed_once(&resources, &athlete).await;
}

#[tokio::test]
async fn a_timed_capture_and_the_export_of_one_ride_are_listed_once() {
    // The capture read with its detail pass: its real start, its own sport.
    let start = evening_start();
    let mut capture = date_only_capture_json();
    capture["start_date"] = json!(start.to_rfc3339());
    capture["sport_type"] = json!("ride");
    let scraper = spawn_mock_scraper_with_activity(capture).await;
    let (imports, _) = imports_serving(one_ride_export(RIDE_ID, start, RIDE_SECONDS));
    let resources = create_test_server_resources_with_archive_imports_and_config(
        imports,
        Some(test_server_config_with_sciotte(&scraper)),
    )
    .await
    .unwrap();
    let athlete = seed_athlete(&resources, "strava-twin-timed").await;
    connect_capture(&resources, &athlete).await;
    import_export(&resources, &athlete).await;

    assert_listed_once(&resources, &athlete).await;
}

// ============================================================================
// Training load
// ============================================================================

/// Real physiology, so every session with a heart rate scores a load.
async fn give_physiology(resources: &Arc<ServerContext>, athlete: &Athlete) {
    let profile = UserPhysiologicalProfile {
        user_id: athlete.user_id,
        vo2_max: Some(52.0),
        resting_hr: Some(50),
        max_hr: Some(190),
        lactate_threshold_percentage: Some(0.85),
        threshold_hr: None,
        age: Some(34),
        weight: Some(72.0),
        fitness_level: FitnessLevel::Advanced,
        primary_sport: SportType::Ride,
        training_experience_years: Some(10),
        ftp_watts: Some(280),
        threshold_pace_sec_per_km: Some(225.0),
        hr_zones: None,
        power_zones: None,
        critical_power_watts: None,
        w_prime_joules: None,
        critical_speed_mps: None,
        d_prime_meters: None,
        threshold_provenance: ThresholdProvenance::default(),
        transport_policy: TransportPolicy::AnyTransport,
        sex: None,
    };
    resources
        .common
        .repos
        .user_physiological_profile
        .upsert_user_physiological_profile(athlete.tenant, athlete.user_id, &profile)
        .await
        .unwrap();
}

/// A ride the usr/pwd connection captured from Strava.
fn captured(id: String, sport: SportType, start: DateTime<Utc>) -> Activity {
    ActivityBuilder::new(
        id,
        "Captured Ride".to_owned(),
        sport,
        start,
        u64::from(RIDE_SECONDS),
        SCIOTTE.to_owned(),
    )
    .source(STRAVA)
    .distance_meters(21_600.0)
    .average_heart_rate(150)
    .build()
}

/// The months of rides before the one both copies hold, every other day.
fn history() -> Vec<Activity> {
    (RIDE_DAYS_AGO + 2..=DEEP_DAYS)
        .filter(|d| d % 2 == 0)
        .map(|d| {
            let day = Utc::now().date_naive() - Duration::days(d);
            let start = Utc.from_utc_datetime(&day.and_hms_opt(7, 0, 0).unwrap());
            captured(format!("1500000{d:04}"), SportType::Ride, start)
        })
        .collect()
}

/// Store what the capture holds for `athlete`: the history, and the
/// date-only ride when `with_ride`.
async fn store_capture(resources: &Arc<ServerContext>, athlete: &Athlete, with_ride: bool) {
    let mut rows = history();
    if with_ride {
        rows.push(captured(
            RIDE_ID.to_owned(),
            SportType::MountainBike,
            ride_day(),
        ));
    }
    let repos = &resources.common.repos;
    repos
        .provider_connections
        .register_connection(
            athlete.user_id,
            athlete.tenant,
            SCIOTTE,
            &ConnectionType::Manual,
            None,
        )
        .await
        .unwrap();
    repos
        .activity_cache
        .upsert_activities(athlete.user_id, &athlete.tenant, SCIOTTE, &rows)
        .await
        .unwrap();
}

/// The persisted rollup over the chronic window ending today.
async fn rollup(resources: &Arc<ServerContext>, athlete: &Athlete) -> Vec<DailyTrainingState> {
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let to = Utc::now().date_naive();
    let from = to - Duration::days(CTL_WINDOW_DAYS);
    compute_and_persist_history(&runtime, athlete.tenant, athlete.user_id, from, to)
        .await
        .expect("the history computes");
    fetch_history_rows(&runtime.data(), athlete.tenant, athlete.user_id, from, to)
        .await
        .unwrap()
}

fn load_on(rollup: &[DailyTrainingState], day: NaiveDate) -> f64 {
    rollup
        .iter()
        .find(|row| row.date == day)
        .unwrap_or_else(|| panic!("the rollup covers {day}"))
        .daily_load
}

#[tokio::test]
async fn a_ride_captured_and_imported_scores_once() {
    let capture_day = ride_day().date_naive();
    let export_day = evening_start().date_naive();

    // The date-only capture alone scores on the athlete's day.
    let capture_resources = create_test_server_resources_with_archive_imports(
        imports_serving(one_ride_export(RIDE_ID, evening_start(), RIDE_SECONDS)).0,
    )
    .await
    .unwrap();
    let captured_only = seed_athlete(&capture_resources, "strava-twin-capture").await;
    give_physiology(&capture_resources, &captured_only).await;
    store_capture(&capture_resources, &captured_only, true).await;
    let alone = rollup(&capture_resources, &captured_only).await;
    assert!(
        load_on(&alone, capture_day) > 0.0,
        "the capture alone scores on {capture_day}"
    );

    // Both copies: the capture, then the export of the same ride.
    let resources = create_test_server_resources_with_archive_imports(
        imports_serving(one_ride_export(RIDE_ID, evening_start(), RIDE_SECONDS)).0,
    )
    .await
    .unwrap();
    let both = seed_athlete(&resources, "strava-twin-both").await;
    give_physiology(&resources, &both).await;
    store_capture(&resources, &both, true).await;
    import_export(&resources, &both).await;
    let held = resources
        .common
        .repos
        .activity_cache
        .get_cached_activity(both.user_id, &both.tenant, STRAVA_ARCHIVE, RIDE_ID)
        .await
        .unwrap();
    assert!(
        held.is_some(),
        "the export's copy is held beside the capture"
    );

    let merged = rollup(&resources, &both).await;
    assert!(
        load_on(&merged, capture_day).abs() < f64::EPSILON,
        "the capture's midnight copy folds into the export's: nothing scores on {capture_day}"
    );
    assert!(
        load_on(&merged, export_day) > 0.0,
        "the ride scores once, at the export's real start on {export_day}"
    );
}

// ============================================================================
// By-id reads (carnet#1243)
// ============================================================================

/// The export's recorded samples of the ride.
fn export_samples() -> TimeSeriesData {
    TimeSeriesData {
        timestamps: vec![0, 1, 2, 3],
        heart_rate: Some(vec![Some(131), Some(134), None, Some(139)]),
        power: Some(vec![Some(190), Some(205), Some(212), Some(220)]),
        cadence: None,
        speed: None,
        altitude: None,
        temperature: None,
        gps_coordinates: Some(vec![
            Some((45.50, -73.60)),
            Some((45.5001, -73.60)),
            Some((45.5002, -73.60)),
            Some((45.5003, -73.60)),
        ]),
        distance: None,
    }
}

/// Hold `rows` under the provider `key` the way the export's projection does:
/// the cached copy, and its samples in the one per-activity sample store.
async fn hold_with_samples(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    key: &str,
    copy: Activity,
    origin: StreamOrigin,
) {
    let repos = &resources.common.repos;
    repos
        .activity_streams
        .upsert_activity_streams(&ActivityStreamsWrite {
            tenant_id: &athlete.tenant,
            user_id: athlete.user_id,
            provider: key,
            activity_id: copy.id(),
            origin,
            source_ref: None,
            streams: &export_samples(),
            synced_at: Utc::now(),
        })
        .await
        .unwrap();
    repos
        .activity_cache
        .upsert_activities(athlete.user_id, &athlete.tenant, key, &[copy])
        .await
        .unwrap();
}

/// The export's copy of the ride: its real start.
fn exported(start: DateTime<Utc>) -> Activity {
    ActivityBuilder::new(
        RIDE_ID.to_owned(),
        "Sortie du soir".to_owned(),
        SportType::Ride,
        start,
        u64::from(RIDE_SECONDS),
        STRAVA_ARCHIVE.to_owned(),
    )
    .distance_meters(21_600.0)
    .build()
}

#[tokio::test]
async fn a_by_id_read_of_a_ride_the_export_stands_for_returns_the_export_samples() {
    let resources = create_test_server_resources_with_archive_imports(
        imports_serving(one_ride_export(RIDE_ID, evening_start(), RIDE_SECONDS)).0,
    )
    .await
    .unwrap();
    let athlete = seed_athlete(&resources, "strava-twin-by-id").await;
    // The usr/pwd capture holds the ride date-only, without samples; no
    // scraper answers, so a read that reached it would fail.
    store_capture(&resources, &athlete, true).await;
    hold_with_samples(
        &resources,
        &athlete,
        STRAVA_ARCHIVE,
        exported(evening_start()),
        StreamOrigin::Archive,
    )
    .await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let tenant = athlete.tenant.to_string();

    assert_eq!(
        by_id_provider(&runtime, athlete.user_id, STRAVA, Some(&tenant), RIDE_ID).await,
        STRAVA_ARCHIVE,
        "the export stands for the ride the capture read date-only"
    );
    let ride =
        fetch_activity_from_provider(&runtime, athlete.user_id, STRAVA, Some(&tenant), RIDE_ID)
            .await
            .expect("the ride is read by its id");
    assert_eq!(ride.provider(), STRAVA_ARCHIVE, "the export's copy is read");
    assert_eq!(
        ride.time_series_data(),
        Some(&export_samples()),
        "the export's stored samples ride with it"
    );

    // An activity only the connection holds still reads from it.
    let only_captured = format!("1500000{:04}", RIDE_DAYS_AGO + 2);
    assert_eq!(
        by_id_provider(
            &runtime,
            athlete.user_id,
            STRAVA,
            Some(&tenant),
            &only_captured
        )
        .await,
        STRAVA,
    );
}

#[tokio::test]
async fn a_by_id_read_of_a_garmin_activity_stays_with_garmin() {
    let resources = create_test_server_resources_with_archive_imports(
        imports_serving(one_ride_export(RIDE_ID, evening_start(), RIDE_SECONDS)).0,
    )
    .await
    .unwrap();
    let athlete = seed_athlete(&resources, "garmin-by-id").await;
    let start = evening_start();
    let pushed = ActivityBuilder::new(
        "19876543210".to_owned(),
        "Pushed Ride".to_owned(),
        SportType::Ride,
        start,
        u64::from(RIDE_SECONDS),
        GARMIN.to_owned(),
    )
    .source(GARMIN)
    .distance_meters(21_600.0)
    .build();
    let captured = ActivityBuilder::new(
        "19876543210".to_owned(),
        "Captured Ride".to_owned(),
        SportType::Ride,
        ride_day(),
        u64::from(RIDE_SECONDS),
        SCIOTTE.to_owned(),
    )
    .source(GARMIN)
    .distance_meters(21_600.0)
    .build();
    hold_with_samples(&resources, &athlete, GARMIN, pushed, StreamOrigin::Push).await;
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(
            athlete.user_id,
            &athlete.tenant,
            SCIOTTE_GARMIN,
            &[captured],
        )
        .await
        .unwrap();
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let tenant = athlete.tenant.to_string();

    assert_eq!(
        by_id_provider(
            &runtime,
            athlete.user_id,
            GARMIN,
            Some(&tenant),
            "19876543210"
        )
        .await,
        GARMIN,
        "the API's copy of a Garmin activity is read where it always was"
    );
}

#[tokio::test]
async fn an_old_ride_the_export_holds_alone_still_reads_from_the_strava_api() {
    // The cache prunes the API's rows past its retention and never the
    // export's, so an old ride can find the export as its only cached copy.
    // The athlete holds a Strava API token: the API stands for its own
    // activity (API > export > capture), whatever the cache still holds.
    let resources = create_test_server_resources_with_archive_imports(
        imports_serving(one_ride_export(RIDE_ID, evening_start(), RIDE_SECONDS)).0,
    )
    .await
    .unwrap();
    let athlete = seed_athlete(&resources, "strava-api-over-export").await;
    hold_with_samples(
        &resources,
        &athlete,
        STRAVA_ARCHIVE,
        exported(evening_start()),
        StreamOrigin::Archive,
    )
    .await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let tenant = athlete.tenant.to_string();
    assert_eq!(
        by_id_provider(&runtime, athlete.user_id, STRAVA, Some(&tenant), RIDE_ID).await,
        STRAVA_ARCHIVE,
        "without a Strava API connection the export is the best copy there is"
    );

    let token = UserOAuthToken::new(
        athlete.user_id,
        tenant.clone(),
        STRAVA.to_owned(),
        "acc".to_owned(),
        Some("ref".to_owned()),
        Some(Utc::now() + Duration::hours(6)),
        Some("read".to_owned()),
    );
    resources
        .common
        .repos
        .oauth_tokens
        .upsert_token(&token)
        .await
        .unwrap();

    assert_eq!(
        by_id_provider(&runtime, athlete.user_id, STRAVA, Some(&tenant), RIDE_ID).await,
        STRAVA,
        "the Strava API outranks the export of its own activity"
    );
    assert!(
        by_id_reader(&runtime, athlete.user_id, STRAVA, Some(&tenant), RIDE_ID)
            .await
            .unwrap()
            .is_none(),
        "the read stays with the elected connection"
    );
}

#[tokio::test]
async fn an_activity_only_the_garmin_export_holds_reads_from_the_export() {
    // Garmin's API is push-only: its by-id read answers from the rows Garmin
    // pushed, so a ride older than the connection, held only in the
    // athlete's Garmin export, has no API copy to redirect to. The export
    // stands for it though the athlete holds a Garmin token.
    let resources = create_test_server_resources_with_archive_imports(
        imports_serving(one_ride_export(RIDE_ID, evening_start(), RIDE_SECONDS)).0,
    )
    .await
    .unwrap();
    let athlete = seed_athlete(&resources, "garmin-export-by-id").await;
    let garmin_id = "19876543211";
    let exported = ActivityBuilder::new(
        garmin_id.to_owned(),
        "Exported Ride".to_owned(),
        SportType::Ride,
        evening_start(),
        u64::from(RIDE_SECONDS),
        GARMIN_ARCHIVE.to_owned(),
    )
    .distance_meters(21_600.0)
    .build();
    hold_with_samples(
        &resources,
        &athlete,
        GARMIN_ARCHIVE,
        exported,
        StreamOrigin::Archive,
    )
    .await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let tenant = athlete.tenant.to_string();
    let token = UserOAuthToken::new(
        athlete.user_id,
        tenant.clone(),
        GARMIN.to_owned(),
        "acc".to_owned(),
        Some("ref".to_owned()),
        Some(Utc::now() + Duration::hours(6)),
        Some("read".to_owned()),
    );
    resources
        .common
        .repos
        .oauth_tokens
        .upsert_token(&token)
        .await
        .unwrap();

    assert_eq!(
        by_id_provider(&runtime, athlete.user_id, GARMIN, Some(&tenant), garmin_id).await,
        GARMIN_ARCHIVE,
        "a push-only API holds no copy of what it never pushed"
    );
}

/// Run `tool_name` for `athlete` the way a turn dispatches it.
async fn run_tool(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    tool_name: &str,
    parameters: Value,
) -> Value {
    let response = UniversalToolExecutor::new(resources.clone())
        .with_scopes(OAuthScope::self_grant())
        .with_transport(Transport::WebApp)
        .execute_tool(UniversalRequest {
            tool_name: tool_name.to_owned(),
            parameters,
            user_id: athlete.user_id.to_string(),
            protocol: "mcp".to_owned(),
            tenant_id: Some(athlete.tenant.to_string()),
        })
        .await
        .expect("the tool dispatches");
    assert!(response.success, "{tool_name} ran: {:?}", response.error);
    response.result.expect("the tool returned a payload")
}

#[tokio::test]
async fn the_per_activity_tools_read_an_uploaded_workout_by_its_id() {
    // Connected to Strava through the usr/pwd capture, whose scraper knows
    // nothing of the file the athlete uploaded: the tools that answer about
    // one activity read the upload's own copy, as the sample tools do.
    let scraper = spawn_mock_scraper_with_activity(date_only_capture_json()).await;
    let resources = create_test_server_resources_with_archive_imports_and_config(
        imports_serving(one_ride_export(RIDE_ID, evening_start(), RIDE_SECONDS)).0,
        Some(test_server_config_with_sciotte(&scraper)),
    )
    .await
    .unwrap();
    let athlete = seed_athlete(&resources, "upload-by-id").await;
    connect_capture(&resources, &athlete).await;
    let upload_id = "9f2c4a1be0d3-0";
    let uploaded = ActivityBuilder::new(
        upload_id.to_owned(),
        "Uploaded Ride".to_owned(),
        SportType::Ride,
        evening_start() - Duration::days(1),
        u64::from(RIDE_SECONDS),
        UPLOAD.to_owned(),
    )
    .distance_meters(25_000.0)
    .average_heart_rate(147)
    .build();
    hold_with_samples(&resources, &athlete, UPLOAD, uploaded, StreamOrigin::Upload).await;

    let intelligence = run_tool(
        &resources,
        &athlete,
        "get_activity_intelligence",
        json!({ "activity_id": upload_id }),
    )
    .await;
    assert!(
        intelligence["auto_selected"].is_null(),
        "the upload itself is analysed, not a substitute: {intelligence}"
    );
    assert_eq!(intelligence["activity_id"], upload_id, "{intelligence}");

    let metrics = run_tool(
        &resources,
        &athlete,
        "calculate_metrics",
        json!({ "activity_id": upload_id, "provider": STRAVA }),
    )
    .await;
    // 25 km in 45 minutes: 108 s/km, the upload's own figures.
    assert_eq!(
        metrics["pace"].as_f64(),
        Some(108.0),
        "metrics are computed from the upload's copy: {metrics}"
    );
}
