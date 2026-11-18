// ABOUTME: query_activities over a seeded store: copies merged across sources, no row cap, the tenant's rows only
// ABOUTME: Coverage per source on the history ladder: a walk owed, a shallow push-only source, a held key, uploads

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The one aggregator over a seeded store (carnet#1485).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// The tool is driven through the dispatch chokepoint, so the transport gates
// and the output backstop run as they do in production. Strava is a loopback
// stand-in (a test double, per the repo's mock rule) answering an empty
// history, so a walk the read asks has nothing to land.
#![cfg(all(
    feature = "tools-data",
    feature = "provider-strava",
    feature = "provider-garmin",
    feature = "health-sync"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers::strava_grant::hold_strava_grant;
use crate::history_walk_api_test::server;

use std::sync::Arc;

use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::constants::provider_capture::current_capture_version;
use pierre_core::models::{
    Activity, ActivityBuilder, ActivityTag, ConnectionType, SportType, TagSource, TaggedBy,
    TenantId, UserOAuthToken,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_database::repositories::{
    ActivityBackfillJobRow, BackfillCoverage, BackfillJobKind, JobProgress,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::activity_query::{parse_query, query_activities, ReadGate};
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::runtime::ToolRuntime;
use secrecy::SecretString;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use uuid::Uuid;

pub(crate) const TOOL: &str = "query_activities";

/// The key the athlete's Strava session files its rides under.
pub(crate) const SESSION: &str = "sciotte";

/// An athlete: their id and tenant.
pub(crate) type Athlete = (Uuid, TenantId);

/// A Strava stand-in answering an empty history, so a walk asked lands
/// nothing.
async fn mock_strava() -> String {
    let app = Router::new()
        .route("/athlete", get(|| async { Json(json!({ "id": 4242 })) }))
        .route("/athlete/activities", get(|| async { Json(json!([])) }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// A server whose Strava provider reaches the stand-in.
pub(crate) async fn store_server() -> Arc<ServerContext> {
    server(&mock_strava().await, 1_000).await
}

/// An athlete with no connection at all.
pub(crate) async fn bare_athlete(resources: &Arc<ServerContext>) -> Athlete {
    let (user_id, _user, tenant) = common::create_test_user_with_plan(
        &resources.agent.database,
        &format!("query-{}@example.com", Uuid::new_v4()),
        "starter",
    )
    .await
    .unwrap();
    (user_id, tenant)
}

/// An athlete holding a live Strava grant.
pub(crate) async fn strava_athlete(resources: &Arc<ServerContext>) -> Athlete {
    let athlete = bare_athlete(resources).await;
    hold_strava_grant(resources, athlete.0, athlete.1).await;
    athlete
}

/// Give the athlete a Garmin connection through its own grant: a provider
/// that pushes its history, which no walk deepens. Without the grant the
/// athlete's Garmin would be read through the session mirror, which a walk
/// does deepen.
pub(crate) async fn connect_garmin(resources: &Arc<ServerContext>, (user_id, tenant): Athlete) {
    let now = Utc::now();
    resources
        .common
        .repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id: tenant.to_string(),
            provider: oauth_providers::GARMIN.to_owned(),
            access_token: SecretString::from(format!("garmin_access_token_{:0>30}", 7)),
            refresh_token: Some(SecretString::from("garmin_refresh")),
            token_type: "Bearer".to_owned(),
            expires_at: Some(now + chrono::Duration::hours(6)),
            scope: None,
            provider_user_id: Some("garmin-4242".to_owned()),
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    resources
        .common
        .repos
        .provider_connections
        .register_connection(
            user_id,
            tenant,
            oauth_providers::GARMIN,
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
}

/// Noon UTC on `y-m-d`.
pub(crate) fn noon(y: i32, m: u32, d: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).single().unwrap()
}

pub(crate) fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// The workout `id` of `sport` that `key` listed, started `at`, of 30 km.
pub(crate) fn workout(key: &str, id: &str, sport: SportType, at: DateTime<Utc>) -> Activity {
    let built = ActivityBuilder::new(id, "Morning ride", sport, at, 3_600, key)
        .distance_meters(30_000.0)
        .elevation_gain(250.0);
    if key == SESSION {
        built.source(oauth_providers::STRAVA).build()
    } else {
        built.build()
    }
}

/// A ride `key` listed, its id its start.
pub(crate) fn ride(key: &str, at: DateTime<Utc>) -> Activity {
    workout(key, &at.timestamp().to_string(), SportType::Ride, at)
}

/// Store `rows` as `key` holds them for the athlete.
pub(crate) async fn store(
    resources: &Arc<ServerContext>,
    (user_id, tenant): Athlete,
    key: &str,
    rows: &[Activity],
) {
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(user_id, &tenant, key, rows)
        .await
        .unwrap();
}

/// Record that `key`'s history is stored to the end of its feed.
pub(crate) async fn walked_to_its_end(
    resources: &Arc<ServerContext>,
    (user_id, tenant): Athlete,
    key: &str,
) {
    resources
        .common
        .repos
        .activity_cache
        .upsert_backfill_coverage(
            user_id,
            &tenant,
            key,
            BackfillCoverage {
                oldest_reached_ts: 0,
                hit_feed_end: true,
                capture_version: current_capture_version(key),
            },
        )
        .await
        .unwrap();
}

/// Record a history walk of `key` owed for the athlete, `done` of `total`
/// months walked.
pub(crate) async fn owe_history_walk(
    resources: &Arc<ServerContext>,
    (user_id, tenant): Athlete,
    key: &str,
    (done, total): (i64, i64),
) {
    let row = ActivityBackfillJobRow {
        id: Uuid::new_v4().to_string(),
        tenant_id: tenant,
        user_id,
        provider: key.to_owned(),
        after_ts: Some(946_684_800),
        before_ts: None,
        fetch_limit: Some(2_000),
        conversation_id: None,
        created_at_ms: Utc::now().timestamp_millis(),
        leased_until_ms: 0,
        attempts: 1,
        lease_token: None,
        kind: BackfillJobKind::HistoryWalk,
        progress: JobProgress {
            units_total: Some(total),
            units_done: done,
            rows_landed: 0,
            secs_per_unit: Some(60.0),
        },
        turn_id: None,
        asked_message_id: None,
        walk_backend: None,
        walk_sport: None,
    };
    assert!(
        resources
            .common
            .repos
            .activity_backfill_jobs
            .record_backfill_job(&row)
            .await
            .unwrap(),
        "the fixture's history walk was not recorded"
    );
}

/// Whether a history walk of `key` is recorded for the athlete.
pub(crate) async fn walk_asked(
    resources: &Arc<ServerContext>,
    (user_id, tenant): Athlete,
    key: &str,
) -> bool {
    resources
        .common
        .repos
        .activity_backfill_jobs
        .history_walk_row(user_id, tenant, key)
        .await
        .unwrap()
        .is_some()
}

fn executor(resources: &Arc<ServerContext>, transport: Transport) -> UniversalToolExecutor {
    let executor = UniversalToolExecutor::new(resources.clone())
        .with_scopes(OAuthScope::self_grant())
        .with_transport(transport);
    if transport.is_first_party() {
        executor
    } else {
        executor.with_caller_writing_reply()
    }
}

/// `tool`'s answer to the athlete over `transport`; panics on a refusal.
pub(crate) async fn call_tool(
    resources: &Arc<ServerContext>,
    transport: Transport,
    (user_id, tenant): Athlete,
    tool: &str,
    params: Value,
) -> Value {
    let response = executor(resources, transport)
        .execute_tool(UniversalRequest {
            tool_name: tool.to_owned(),
            parameters: params,
            user_id: user_id.to_string(),
            protocol: "test".to_owned(),
            tenant_id: Some(tenant.to_string()),
        })
        .await
        .unwrap_or_else(|e| panic!("{tool} raised: {e}"));
    assert!(
        response.success,
        "bug: {tool} failed: {:?} {:?}",
        response.error, response.result
    );
    response.result.expect("a successful call answers")
}

/// `query_activities` from the athlete's own web app.
pub(crate) async fn query(
    resources: &Arc<ServerContext>,
    athlete: Athlete,
    params: Value,
) -> Value {
    call_tool(resources, Transport::WebApp, athlete, TOOL, params).await
}

/// The entry of `coverage_by_source` naming `source`.
pub(crate) fn source_entry<'a>(payload: &'a Value, source: &str) -> &'a Value {
    payload["coverage_by_source"]
        .as_array()
        .expect("coverage_by_source is a list")
        .iter()
        .find(|entry| entry["source"] == source)
        .unwrap_or_else(|| panic!("bug: {source} is not listed: {payload}"))
}

#[tokio::test]
async fn one_workout_held_by_three_sources_counts_once() {
    let resources = store_server().await;
    let athlete = strava_athlete(&resources).await;
    walked_to_its_end(&resources, athlete, oauth_providers::STRAVA).await;
    let at = noon(2024, 6, 1);
    store(
        &resources,
        athlete,
        oauth_providers::STRAVA,
        &[ride(oauth_providers::STRAVA, at)],
    )
    .await;
    store(&resources, athlete, SESSION, &[ride(SESSION, at)]).await;
    store(
        &resources,
        athlete,
        oauth_providers::GARMIN,
        &[workout(
            oauth_providers::GARMIN,
            "garmin-9",
            SportType::Ride,
            at,
        )],
    )
    .await;

    let payload = query(&resources, athlete, json!({ "from": "2024-01-01" })).await;

    assert_eq!(
        payload["totals"]["count"], 1,
        "bug: the Strava API copy, its sciotte mirror and its Garmin twin are one workout: {payload}"
    );
    assert_eq!(
        payload["totals"]["distance_km"], 30.0,
        "bug: one workout's distance is counted once: {payload}"
    );
    let copies: u64 = payload["coverage_by_source"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["copies"].as_u64().unwrap())
        .sum();
    assert_eq!(
        copies, 3,
        "bug: the sources list the copies they hold before the merge: {payload}"
    );
}

#[tokio::test]
async fn a_workout_whose_copies_straddle_new_year_counts_once_in_its_year() {
    let resources = store_server().await;
    let athlete = strava_athlete(&resources).await;
    walked_to_its_end(&resources, athlete, oauth_providers::STRAVA).await;
    let strava_at = Utc
        .with_ymd_and_hms(2024, 12, 31, 23, 59, 40)
        .single()
        .unwrap();
    let garmin_at = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 10).single().unwrap();
    store(
        &resources,
        athlete,
        oauth_providers::STRAVA,
        &[ride(oauth_providers::STRAVA, strava_at)],
    )
    .await;
    store(
        &resources,
        athlete,
        oauth_providers::GARMIN,
        &[workout(
            oauth_providers::GARMIN,
            "garmin-ny",
            SportType::Ride,
            garmin_at,
        )],
    )
    .await;

    let payload = query(
        &resources,
        athlete,
        json!({ "from": "2024-01-01", "to": "2025-12-31", "group_by": ["year"] }),
    )
    .await;

    assert_eq!(
        payload["totals"]["count"], 1,
        "bug: a workout read by two year chunks is counted twice: {payload}"
    );
    let groups = payload["groups"].as_array().unwrap();
    assert_eq!(
        groups.len(),
        1,
        "bug: one workout lands in one year only: {payload}"
    );
}

#[tokio::test]
async fn another_tenant_reads_none_of_the_athletes_rows() {
    let resources = store_server().await;
    let athlete = strava_athlete(&resources).await;
    store(
        &resources,
        athlete,
        oauth_providers::STRAVA,
        &[ride(oauth_providers::STRAVA, noon(2024, 6, 1))],
    )
    .await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let asked = parse_query(&json!({}), day(2026, 11, 17)).unwrap();

    let own = query_activities(&runtime, athlete.0, athlete.1, &asked, ReadGate::Model)
        .await
        .unwrap();
    let other = query_activities(
        &runtime,
        athlete.0,
        TenantId::from_uuid(Uuid::new_v4()),
        &asked,
        ReadGate::Model,
    )
    .await
    .unwrap();

    assert_eq!(
        own.totals.count,
        Some(1),
        "bug: the athlete's own tenant reads their ride"
    );
    assert_eq!(
        other.totals.count,
        Some(0),
        "bug: the same user id in another tenant read the athlete's rows"
    );
}

#[tokio::test]
async fn every_row_of_a_long_year_is_counted() {
    let resources = store_server().await;
    let athlete = strava_athlete(&resources).await;
    walked_to_its_end(&resources, athlete, oauth_providers::STRAVA).await;
    let first = noon(2023, 1, 1);
    let rows: Vec<Activity> = (0..2_600_i64)
        .map(|i| {
            ride(
                oauth_providers::STRAVA,
                first + chrono::Duration::hours(3 * i),
            )
        })
        .collect();
    store(&resources, athlete, oauth_providers::STRAVA, &rows).await;

    let payload = query(
        &resources,
        athlete,
        json!({ "from": "2023-01-01", "to": "2023-12-31" }),
    )
    .await;

    assert_eq!(
        payload["totals"]["count"], 2_600,
        "bug: a year of 2,600 workouts was cut by a row cap: {}",
        payload["totals"]
    );
}

#[tokio::test]
async fn a_walk_owed_and_not_reached_says_so_with_its_progress() {
    let resources = store_server().await;
    let athlete = strava_athlete(&resources).await;
    owe_history_walk(&resources, athlete, oauth_providers::STRAVA, (3, 40)).await;
    store(
        &resources,
        athlete,
        oauth_providers::STRAVA,
        &[ride(oauth_providers::STRAVA, noon(2025, 6, 1))],
    )
    .await;

    let payload = query(&resources, athlete, json!({ "from": "2020-01-01" })).await;

    let coverage = &payload["coverage"];
    assert_eq!(
        coverage["incomplete_reason"], "history_walk_running",
        "bug: a window the walk has not reached reads as complete: {coverage}"
    );
    assert_eq!(
        coverage["pending"]["months_done"], 3,
        "bug: the walk's own progress is reported: {coverage}"
    );
}

#[tokio::test]
async fn a_never_walked_source_is_asked_its_walk_and_said_running() {
    let resources = store_server().await;
    let athlete = strava_athlete(&resources).await;
    store(
        &resources,
        athlete,
        oauth_providers::STRAVA,
        &[ride(oauth_providers::STRAVA, noon(2025, 6, 1))],
    )
    .await;

    let payload = query(&resources, athlete, json!({ "from": "2020-01-01" })).await;

    assert_eq!(
        payload["coverage"]["pending"]["kind"], "history_not_walked",
        "bug: an unwalked source reads as a shallow history the guard lets through: {payload}"
    );
    assert!(
        walk_asked(&resources, athlete, oauth_providers::STRAVA).await,
        "bug: the Strava walk was not asked"
    );
}

#[tokio::test]
async fn the_walk_asked_is_the_counted_sources_alone() {
    let resources = store_server().await;
    let athlete = strava_athlete(&resources).await;
    store(
        &resources,
        athlete,
        oauth_providers::UPLOAD,
        &[ride(oauth_providers::UPLOAD, noon(2025, 3, 1))],
    )
    .await;
    store(
        &resources,
        athlete,
        oauth_providers::STRAVA,
        &[ride(oauth_providers::STRAVA, noon(2025, 6, 1))],
    )
    .await;

    let uploads = query(
        &resources,
        athlete,
        json!({ "from": "2020-01-01", "provider": "upload" }),
    )
    .await;
    assert!(
        !walk_asked(&resources, athlete, oauth_providers::STRAVA).await,
        "bug: a count of the uploads asked the Strava walk"
    );
    assert_eq!(
        uploads["totals"]["count"], 1,
        "bug: provider upload reads the upload key alone: {uploads}"
    );
    assert_eq!(
        uploads["coverage"]["complete"], true,
        "bug: the uploads hold what the athlete brought: {uploads}"
    );

    query(
        &resources,
        athlete,
        json!({ "from": "2020-01-01", "provider": "strava" }),
    )
    .await;
    assert!(
        walk_asked(&resources, athlete, oauth_providers::STRAVA).await,
        "bug: a count of Strava did not ask Strava's walk"
    );
}

#[tokio::test]
async fn an_upload_only_athlete_is_complete() {
    let resources = store_server().await;
    let athlete = bare_athlete(&resources).await;
    store(
        &resources,
        athlete,
        oauth_providers::UPLOAD,
        &[ride(oauth_providers::UPLOAD, noon(2025, 3, 1))],
    )
    .await;

    let payload = query(&resources, athlete, json!({ "from": "2020-01-01" })).await;

    assert_eq!(
        payload["coverage"]["complete"], true,
        "bug: an athlete holding only uploads reads short: {payload}"
    );
    assert_eq!(payload["totals"]["count"], 1, "{payload}");
}

/// A window from the calendar's first year pages only the years the store
/// holds rows in, never the empty millennia before them, and a window of
/// that first day alone answers nothing rather than failing.
#[tokio::test]
async fn a_window_from_the_calendars_first_year_reads_the_stored_years() {
    let resources = store_server().await;
    let athlete = strava_athlete(&resources).await;
    walked_to_its_end(&resources, athlete, oauth_providers::STRAVA).await;
    let rows = [
        ride(oauth_providers::STRAVA, noon(2024, 6, 1)),
        ride(oauth_providers::STRAVA, noon(2025, 6, 1)),
    ];
    store(&resources, athlete, oauth_providers::STRAVA, &rows).await;

    let all = query(
        &resources,
        athlete,
        json!({ "from": "-262143-01-01", "to": "2026-01-01" }),
    )
    .await;
    assert_eq!(
        all["totals"]["count"], 2,
        "bug: a window from the calendar's first year lost a stored workout: {all}"
    );

    let first_day = query(
        &resources,
        athlete,
        json!({ "from": "-262143-01-01", "to": "-262143-01-01" }),
    )
    .await;
    assert_eq!(
        first_day["totals"]["count"], 0,
        "bug: the calendar's first day counted a workout: {first_day}"
    );
}

#[tokio::test]
async fn nothing_stored_says_so() {
    let resources = store_server().await;
    let athlete = bare_athlete(&resources).await;
    connect_garmin(&resources, athlete).await;

    let payload = query(&resources, athlete, json!({})).await;

    assert_eq!(
        payload["coverage"]["incomplete_reason"], "no_stored_activities",
        "bug: an empty store reads as a true zero: {payload}"
    );
    assert_eq!(payload["totals"]["count"], 0, "{payload}");
}

#[tokio::test]
async fn a_push_only_source_alone_says_where_its_history_starts() {
    let resources = store_server().await;
    let athlete = bare_athlete(&resources).await;
    connect_garmin(&resources, athlete).await;
    store(
        &resources,
        athlete,
        oauth_providers::GARMIN,
        &[workout(
            oauth_providers::GARMIN,
            "g-1",
            SportType::Ride,
            noon(2023, 4, 2),
        )],
    )
    .await;

    let payload = query(&resources, athlete, json!({ "from": "2020-01-01" })).await;

    let coverage = &payload["coverage"];
    assert_eq!(
        coverage["incomplete_reason"], "shallow_history",
        "bug: a push-only history from 2023 read as a whole window from 2020: {coverage}"
    );
    assert_eq!(
        coverage["trustworthy_from"], "2023-04-02",
        "bug: the answer does not say where the pushes start: {coverage}"
    );
}

#[tokio::test]
async fn a_complete_walked_source_stands_beside_a_shallow_push_only_one() {
    let resources = store_server().await;
    let athlete = strava_athlete(&resources).await;
    walked_to_its_end(&resources, athlete, oauth_providers::STRAVA).await;
    connect_garmin(&resources, athlete).await;
    store(
        &resources,
        athlete,
        oauth_providers::STRAVA,
        &[ride(oauth_providers::STRAVA, noon(2015, 5, 1))],
    )
    .await;
    store(
        &resources,
        athlete,
        oauth_providers::GARMIN,
        &[workout(
            oauth_providers::GARMIN,
            "g-2",
            SportType::Run,
            noon(2023, 4, 2),
        )],
    )
    .await;

    let payload = query(&resources, athlete, json!({})).await;

    assert_eq!(
        payload["coverage"]["complete"], true,
        "bug: a push-only source no wait deepens made the all-time count short: {payload}"
    );
    let garmin = source_entry(&payload, "garmin");
    assert_eq!(
        garmin["coverage"]["incomplete_reason"], "shallow_history",
        "bug: the push-only source does not say its history is short: {garmin}"
    );
    assert_eq!(
        garmin["coverage"]["trustworthy_from"], "2023-04-02",
        "bug: the push-only source does not say where it starts: {garmin}"
    );
    assert_eq!(garmin["copies"], 1, "{garmin}");
    assert_eq!(
        source_entry(&payload, "strava")["copies"],
        1,
        "bug: Strava's copies are not listed: {payload}"
    );
}

#[tokio::test]
async fn a_key_no_connection_names_is_counted_and_listed() {
    let resources = store_server().await;
    let athlete = strava_athlete(&resources).await;
    walked_to_its_end(&resources, athlete, oauth_providers::STRAVA).await;
    store(
        &resources,
        athlete,
        oauth_providers::STRAVA,
        &[ride(oauth_providers::STRAVA, noon(2016, 5, 1))],
    )
    .await;
    // Rows a former Garmin connection left behind.
    store(
        &resources,
        athlete,
        oauth_providers::GARMIN,
        &[workout(
            oauth_providers::GARMIN,
            "g-3",
            SportType::Run,
            noon(2024, 2, 3),
        )],
    )
    .await;

    let payload = query(&resources, athlete, json!({ "from": "2016-01-01" })).await;

    assert_eq!(
        payload["totals"]["count"], 2,
        "bug: the rows a former connection left are not counted: {payload}"
    );
    let held = source_entry(&payload, "garmin");
    assert_eq!(
        held["coverage"]["incomplete_reason"], "shallow_history",
        "bug: a held key is judged as a source no walk deepens: {held}"
    );
    assert_eq!(
        payload["coverage"]["complete"], true,
        "bug: a held key made a complete Strava count short: {payload}"
    );
}

#[tokio::test]
async fn a_matched_ride_strava_never_held_is_an_unknown_segment_never_unread() {
    let resources = store_server().await;
    let athlete = strava_athlete(&resources).await;
    walked_to_its_end(&resources, athlete, oauth_providers::STRAVA).await;
    store(
        &resources,
        athlete,
        oauth_providers::GARMIN,
        &[workout(
            oauth_providers::GARMIN,
            "g-4",
            SportType::Ride,
            noon(2025, 5, 4),
        )],
    )
    .await;

    let payload = query(
        &resources,
        athlete,
        json!({ "from": "2025-01-01", "has_segment": "La Bête" }),
    )
    .await;

    assert_eq!(
        payload["unknown"]["has_segment"], 1,
        "bug: a ride no detail walk can read is not counted unknown: {payload}"
    );
    assert_eq!(
        payload["totals"]["count"], 0,
        "bug: an unknown is never a match: {payload}"
    );
    assert_ne!(
        payload["coverage"]["incomplete_reason"], "segment_detail_unread",
        "bug: a ride Strava never held made the window's segments unread: {payload}"
    );
}

#[tokio::test]
async fn workout_type_filters_judge_each_tag_by_what_its_source_can_say() {
    let resources = store_server().await;
    let athlete = strava_athlete(&resources).await;
    walked_to_its_end(&resources, athlete, oauth_providers::STRAVA).await;
    let at = |d: u32| noon(2025, 5, d);
    let tagged = |id: &str, d: u32, tags: Vec<ActivityTag>| {
        ActivityBuilder::new(
            id,
            "Tagged run",
            SportType::Run,
            at(d),
            1_800,
            oauth_providers::STRAVA,
        )
        .tags(
            tags.into_iter()
                .map(|tag| TaggedBy::new(tag, TagSource::ProviderTag))
                .collect(),
        )
        .build()
    };
    let bare = ActivityBuilder::new(
        "bare",
        "Run, no tag read",
        SportType::Run,
        at(5),
        1_800,
        oauth_providers::STRAVA,
    )
    .build();
    let coded_race = ActivityBuilder::new(
        "coded",
        "Race by code",
        SportType::Run,
        at(6),
        1_800,
        oauth_providers::STRAVA,
    )
    .workout_type(1)
    .build();
    store(
        &resources,
        athlete,
        oauth_providers::STRAVA,
        &[
            tagged("commute", 1, vec![ActivityTag::Commute]),
            tagged("recovery", 2, vec![ActivityTag::Recovery]),
            tagged("none", 3, Vec::new()),
            tagged("race-tag", 4, vec![ActivityTag::Race]),
            bare,
            coded_race,
        ],
    )
    .await;

    for (asked, matched, unknown) in [("commute", 1, 2), ("recovery", 1, 2), ("race", 2, 1)] {
        let payload = query(
            &resources,
            athlete,
            json!({ "from": "2025-01-01", "workout_type": [asked] }),
        )
        .await;
        assert_eq!(
            payload["totals"]["count"], matched,
            "bug: {asked} matched the wrong workouts: {payload}"
        );
        assert_eq!(
            payload["unknown"]["workout_type"], unknown,
            "bug: {asked} counted the wrong unknowns: {payload}"
        );
    }
}
