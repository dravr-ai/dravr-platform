// ABOUTME: The track walk: every outdoor ride whose track is unsettled is read once for it, detailed before or never
// ABOUTME: Newest first, after any owed detail walk, in the detail walk's day budget, never a ride whose track is settled

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Track walks (carnet#1679).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// A detail read keeps the route it carried as the ride's track, but the rides
// a detail walk read before it did, and the rides no detail read reached (a
// session's list row carries no route), hold no track. The track walk reads
// each once through the athlete's session. The session reader is a loopback
// stand-in (a test double, per the repo's mock rule) that records every
// detail read, in order, so "once", "newest first" and "the detail walk
// first" are its log.
#![cfg(all(
    feature = "provider-strava",
    feature = "provider-sciotte",
    feature = "provider-garmin",
    feature = "tools-data",
    feature = "health-sync"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::detail_walk_test::{rows_under, since, store_rides, DAY, WALK_DEADLINE};
use crate::helpers::sciotte_mock::seed_sciotte_session;
use crate::history_walk_api_test::{open_window, steps_settled_under, with_window};
use crate::history_walk_test::{age_job_rows, paused_retries_due};

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use dravr_barrage::parse::polyline::encode_polyline;
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{ActivityBuilder, ConnectionType, SportType, TenantId};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::{
    ActivityBackfillJobRow, ActivityDetail, BackfillJobKind, BackfillJobState, DetailWalkRow,
    JobProgress, StoredRouteTrack,
};
use pierre_fitness_compute::route_track::RouteTrack;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::request_budget::{
    budget_counter_key, budget_period, detail_walk_athlete_counter_key, detail_walk_counter_key,
    ONE_DAY, PLATFORM_SCOPE,
};
use pierre_services::live_activities::WriteOrigin;
use pierre_tool_runtime::activity_backfill::detail_walk::{
    ask_detail_walk, ask_track_walk, DetailWalkAsked, TrackWalkAsked,
};
use pierre_tool_runtime::activity_backfill::MAX_BACKFILL_ATTEMPTS;
use pierre_tool_runtime::activity_backfill_resume::resume_backfill_jobs;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::time;
use uuid::Uuid;

/// The key the athlete's session files its rides under.
const SESSION: &str = "sciotte";

/// The trailhead every fixture ride starts at.
const TRAILHEAD: (f64, f64) = (45.6012, -73.8044);

/// What the session reader stand-in answers for a ride's detail read. Every answer
/// carries the ride's segment block, as the service's 2.1 reads it: a re-read
/// keeps the ride's efforts read.
#[derive(Clone)]
enum Detail {
    /// A detail carrying the loop as its route.
    Route,
    /// A detail whose route has no point: a ride with no GPS.
    NoGps,
    /// A detail with no `route` key: the page was read before its map.
    NoRoute,
    /// A `503`: the service is down.
    Fails,
}

/// The session reader stand-in: each ride's answer, and every detail read it saw.
#[derive(Default)]
struct SessionReader {
    answers: Mutex<HashMap<String, Detail>>,
    reads: Mutex<Vec<String>>,
}

impl SessionReader {
    fn answer(&self, start: i64, detail: Detail) {
        self.answers
            .lock()
            .unwrap()
            .insert(start.to_string(), detail);
    }

    /// Every ride read, in the order the reads came.
    fn reads(&self) -> Vec<String> {
        self.reads.lock().unwrap().clone()
    }

    /// How many times the ride started at `start` was read.
    fn reads_of(&self, start: i64) -> usize {
        let id = start.to_string();
        self.reads().iter().filter(|read| **read == id).count()
    }
}

/// A ~9 km loop north of the trailhead.
fn loop_track() -> Vec<(f64, f64)> {
    (0..900)
        .map(|i| {
            let step = f64::from(i);
            (
                0.0001f64.mul_add(step, TRAILHEAD.0),
                0.0003f64.mul_add((step * 0.17).sin(), TRAILHEAD.1),
            )
        })
        .collect()
}

/// `GET /api/activities/{id}`: the ride whose id is its start, as
/// [`SessionReader::answer`] set it.
async fn detail(
    State(reader): State<Arc<SessionReader>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    reader.reads.lock().unwrap().push(id.clone());
    let answer = reader
        .answers
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .unwrap_or(Detail::NoRoute);
    let start = id
        .parse::<i64>()
        .ok()
        .and_then(|at| Timestamp::from_epoch_seconds(at).ok())
        .unwrap_or_else(Timestamp::now);
    let mut body = json!({
        "id": id,
        "name": "Morning ride",
        "sport_type": "ride",
        "start_date": start.to_rfc3339(),
        "duration_seconds": 3_600,
        "distance_meters": 20_000.0,
        "provider": "strava",
        "segment_efforts": []
    });
    let coordinates = |points: Vec<(f64, f64)>| {
        json!({
            "coordinates": points
                .iter()
                .map(|&(lat, lon)| json!([lat, lon]))
                .collect::<Vec<_>>()
        })
    };
    match answer {
        Detail::Fails => return Err(StatusCode::SERVICE_UNAVAILABLE),
        Detail::NoRoute => {}
        Detail::NoGps => body["route"] = coordinates(Vec::new()),
        Detail::Route => body["route"] = coordinates(loop_track()),
    }
    Ok(Json(body))
}

async fn spawn_session_reader() -> (String, Arc<SessionReader>) {
    let reader = Arc::new(SessionReader::default());
    let app = Router::new()
        .route(
            "/auth/import-session",
            post(|| async { Json(json!({ "session_id": "cap-verified-session" })) }),
        )
        .route("/api/activities/{id}", get(detail))
        .with_state(Arc::clone(&reader));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), reader)
}

/// A server reaching the session reader stand-in, its session walks counted in the
/// walk windows of its Strava app.
async fn session_server(reader_url: &str) -> Arc<ServerContext> {
    let mut config = common::test_server_config_with_sciotte(reader_url);
    common::set_server_app(&mut config, "strava", "walk_client", "walk_secret");
    let resources = common::create_test_server_resources_with_config(config)
        .await
        .unwrap();
    with_window(&resources, 400)
}

/// An athlete, in a tenant of their own, holding a live signed-in session.
pub(crate) async fn session_athlete(
    resources: &Arc<ServerContext>,
    label: &str,
) -> (Uuid, TenantId) {
    let (user_id, _user, tenant) = common::create_test_user_with_plan(
        &resources.agent.database,
        &format!("{label}-{}@example.com", Uuid::new_v4()),
        "starter",
    )
    .await
    .unwrap();
    seed_sciotte_session(resources, user_id, tenant).await;
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, SESSION, &ConnectionType::Manual, None)
        .await
        .unwrap();
    (user_id, tenant)
}

/// Store `starts` as rides the session listed and a detail read reached
/// before it kept the route: their efforts (none) stored, no track.
async fn store_walked_rides(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    starts: &[i64],
) {
    store_rides(resources, user_id, tenant, SESSION, starts).await;
    let detail = ActivityDetail {
        segment_efforts: Some(Vec::new()),
        ..ActivityDetail::default()
    };
    for start in starts {
        let stored = resources
            .common
            .repos
            .activity_cache
            .store_activity_detail(user_id, &tenant, SESSION, &start.to_string(), &detail, None)
            .await
            .unwrap();
        assert!(
            stored,
            "bug: the fixture's detail found no row to sit beside"
        );
    }
}

/// What is stored for the athlete's ride started at `start`.
async fn track_of(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    start: i64,
) -> Option<StoredRouteTrack> {
    resources
        .common
        .repos
        .activity_route_tracks
        .get_route_track(&tenant, user_id, SESSION, &start.to_string())
        .await
        .unwrap()
}

/// Store `track` as the stored route answer of the ride started at `start`.
async fn settle_track(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    start: i64,
    track: &StoredRouteTrack,
) {
    resources
        .common
        .repos
        .activity_route_tracks
        .upsert_route_track(&tenant, user_id, SESSION, &start.to_string(), track)
        .await
        .unwrap();
}

/// The athlete's `kind` walk row under the session.
async fn walk_row(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    kind: BackfillJobKind,
) -> Option<DetailWalkRow> {
    resources
        .common
        .repos
        .activity_backfill_jobs
        .detail_walk_row(user_id, tenant, SESSION, kind)
        .await
        .unwrap()
}

/// Run the athlete's walks the sweep's way, one step a pass, in a fresh
/// window each pass, until no detail walk is owed and the track walk ended;
/// the track walk's row as it ended.
async fn track_walk_ended(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> DetailWalkRow {
    let deadline = Instant::now() + WALK_DEADLINE;
    loop {
        for kind in [BackfillJobKind::DetailWalk, BackfillJobKind::TrackWalk] {
            steps_settled_under(resources, user_id, tenant, SESSION, kind, deadline).await;
        }
        let owed = |row: &Option<DetailWalkRow>| {
            row.as_ref()
                .is_some_and(|row| row.state == BackfillJobState::Owed)
        };
        let detail = walk_row(resources, user_id, tenant, BackfillJobKind::DetailWalk).await;
        let track = walk_row(resources, user_id, tenant, BackfillJobKind::TrackWalk).await;
        if !owed(&detail) && !owed(&track) {
            return track.expect("bug: no track walk row was recorded");
        }
        assert!(
            Instant::now() < deadline,
            "bug: the walks did not end within {WALK_DEADLINE:?} (track: {track:?})"
        );
        open_window(resources).await;
        paused_retries_due(resources, user_id).await;
        age_job_rows(resources, user_id).await;
        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        resume_backfill_jobs(runtime).await.unwrap();
        time::sleep(Duration::from_millis(50)).await;
    }
}

/// The count of the walk day `key` holds.
async fn day_count(resources: &Arc<ServerContext>, key: &str) -> i64 {
    resources
        .common
        .repos
        .usage_counters
        .get_counter(
            PLATFORM_SCOPE,
            PLATFORM_SCOPE,
            key,
            &budget_period(Timestamp::now(), ONE_DAY),
        )
        .await
        .map_or(0, |counter| counter.value)
}

/// A ride the session's detail walk read before it kept the route is read
/// once more, newest first, and gets its track: drawn, or settled no GPS; a
/// read with no route settles nothing and is still not read again. The reads
/// are counted in the detail walk's own day windows, and another athlete's
/// copies of the same rides are neither read nor written.
#[tokio::test]
async fn a_ride_read_for_its_efforts_before_its_route_was_kept_is_read_once_more() {
    let (reader_url, reader) = spawn_session_reader().await;
    let resources = session_server(&reader_url).await;
    let (user_id, tenant) = session_athlete(&resources, "track-walk").await;
    let (other_user, other_tenant) = session_athlete(&resources, "track-walk-other").await;
    let now = Timestamp::now().epoch_seconds();
    let (unrendered, no_gps, drawn) = (now - 30 * DAY, now - 20 * DAY, now - 10 * DAY);
    reader.answer(drawn, Detail::Route);
    reader.answer(no_gps, Detail::NoGps);
    reader.answer(unrendered, Detail::NoRoute);
    store_walked_rides(&resources, user_id, tenant, &[unrendered, no_gps, drawn]).await;
    store_walked_rides(
        &resources,
        other_user,
        other_tenant,
        &[unrendered, no_gps, drawn],
    )
    .await;
    open_window(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert_eq!(
        ask_track_walk(&runtime, user_id, tenant, oauth_providers::STRAVA).await,
        TrackWalkAsked::NotAsked,
        "bug: a grant, whose detail carries no route, is walked for tracks"
    );
    assert_eq!(
        ask_track_walk(&runtime, user_id, tenant, SESSION).await,
        TrackWalkAsked::Started,
        "bug: the session's track walk was not asked"
    );
    let ended = track_walk_ended(&resources, user_id, tenant).await;
    assert_eq!(
        ended.state,
        BackfillJobState::Walked,
        "bug: the track walk did not walk the whole history: {ended:?}"
    );
    let order = vec![
        drawn.to_string(),
        no_gps.to_string(),
        unrendered.to_string(),
    ];
    assert_eq!(
        reader.reads(),
        order,
        "bug: each walked ride is not read exactly once, newest first"
    );
    match track_of(&resources, user_id, tenant, drawn).await {
        Some(StoredRouteTrack::Drawn { .. }) => {}
        other => panic!("bug: the re-read ride's route was not stored as its track: {other:?}"),
    }
    assert_eq!(
        track_of(&resources, user_id, tenant, no_gps).await,
        Some(StoredRouteTrack::Unavailable {
            source: "streams".to_owned(),
            reason: "no_gps".to_owned(),
            expires_at: None,
        }),
        "bug: the re-read of an empty route did not settle no_gps"
    );
    assert_eq!(
        track_of(&resources, user_id, tenant, unrendered).await,
        None,
        "bug: a re-read that carried no route stored a route answer"
    );
    for start in [unrendered, no_gps, drawn] {
        assert_eq!(
            track_of(&resources, other_user, other_tenant, start).await,
            None,
            "bug: the athlete's re-read wrote another athlete's track"
        );
    }
    let app = budget_counter_key(oauth_providers::STRAVA, "walk_client");
    assert_eq!(
        day_count(&resources, &detail_walk_counter_key(&app)).await,
        3,
        "bug: the re-reads were not counted in the walk's shared day"
    );
    assert_eq!(
        day_count(&resources, &detail_walk_athlete_counter_key(&app, user_id)).await,
        3,
        "bug: the re-reads were not counted in the athlete's walk day"
    );
    let rows = rows_under(&resources, user_id, tenant, SESSION).await;
    assert!(
        rows.iter().all(|row| row.segment_read),
        "bug: a re-read cleared the efforts the ride's first read stored: {rows:?}"
    );

    // Walked once: no ride is read for its track again, the unrendered one
    // included.
    assert_eq!(
        ask_track_walk(&runtime, user_id, tenant, SESSION).await,
        TrackWalkAsked::Standing,
        "bug: a connection's walked track walk was asked again"
    );
    track_walk_ended(&resources, user_id, tenant).await;
    assert_eq!(
        reader.reads(),
        order,
        "bug: a ride was read for its track a second time"
    );
}

/// A ride with a drawn track, a settled miss, or a route overview on a copy
/// is never read for its track, whether a detail read reached it or not. An
/// answer that settled nothing is read.
#[tokio::test]
async fn a_ride_whose_track_is_settled_is_never_read_again() {
    let (reader_url, reader) = spawn_session_reader().await;
    let resources = session_server(&reader_url).await;
    let (user_id, tenant) = session_athlete(&resources, "track-walk-settled").await;
    let now = Timestamp::now().epoch_seconds();
    let (drawn, no_gps, overview, never_read_drawn, unavailable) = (
        now - 25 * DAY,
        now - 20 * DAY,
        now - 15 * DAY,
        now - 10 * DAY,
        now - 5 * DAY,
    );
    reader.answer(unavailable, Detail::Route);
    store_walked_rides(
        &resources,
        user_id,
        tenant,
        &[drawn, no_gps, overview, unavailable],
    )
    .await;
    // No detail read reached it, and its track is drawn (a Home map read).
    store_rides(&resources, user_id, tenant, SESSION, &[never_read_drawn]).await;
    let track = RouteTrack::from_overview(&loop_track()).unwrap();
    settle_track(
        &resources,
        user_id,
        tenant,
        never_read_drawn,
        &StoredRouteTrack::Drawn {
            source: "streams".to_owned(),
            track_json: serde_json::to_string(&track).unwrap(),
        },
    )
    .await;
    settle_track(
        &resources,
        user_id,
        tenant,
        drawn,
        &StoredRouteTrack::Drawn {
            source: "streams".to_owned(),
            track_json: serde_json::to_string(&track).unwrap(),
        },
    )
    .await;
    settle_track(
        &resources,
        user_id,
        tenant,
        no_gps,
        &StoredRouteTrack::Unavailable {
            source: "streams".to_owned(),
            reason: "no_gps".to_owned(),
            expires_at: None,
        },
    )
    .await;
    settle_track(
        &resources,
        user_id,
        tenant,
        unavailable,
        &StoredRouteTrack::Unavailable {
            source: "streams".to_owned(),
            reason: "unavailable".to_owned(),
            expires_at: Some(Timestamp::now() + Elapsed::hours(1)),
        },
    )
    .await;
    // The grant's copy of one ride carries its route overview.
    let listed = ActivityBuilder::new(
        overview.to_string(),
        "Morning ride",
        SportType::Ride,
        Timestamp::from_epoch_seconds(overview).unwrap(),
        3_600,
        oauth_providers::STRAVA,
    )
    .summary_polyline(encode_polyline(&loop_track()))
    .build();
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    runtime
        .live_activities()
        .ingest(
            user_id,
            tenant,
            oauth_providers::STRAVA,
            &[listed],
            Timestamp::now(),
            WriteOrigin::Walk,
        )
        .await
        .unwrap();
    open_window(&resources).await;

    assert_eq!(
        ask_track_walk(&runtime, user_id, tenant, SESSION).await,
        TrackWalkAsked::Started,
        "bug: the session's track walk was not asked"
    );
    let ended = track_walk_ended(&resources, user_id, tenant).await;
    assert_eq!(
        ended.state,
        BackfillJobState::Walked,
        "bug: the track walk did not end: {ended:?}"
    );
    assert_eq!(
        reader.reads(),
        vec![unavailable.to_string()],
        "bug: the track walk read a ride whose track is settled"
    );
    match track_of(&resources, user_id, tenant, unavailable).await {
        Some(StoredRouteTrack::Drawn { .. }) => {}
        other => panic!("bug: an unsettled answer was not replaced by the read: {other:?}"),
    }
}

/// A ride no detail read ever reached (a session's list row carries no
/// route, and no detail walk was asked over it) is read by the track walk,
/// newest first, once, and keeps the track its read carried: drawn, or
/// settled no GPS. Its efforts are stored by the same read.
#[tokio::test]
async fn a_ride_no_detail_read_reached_is_read_for_its_track() {
    let (reader_url, reader) = spawn_session_reader().await;
    let resources = session_server(&reader_url).await;
    let (user_id, tenant) = session_athlete(&resources, "track-walk-never").await;
    let now = Timestamp::now().epoch_seconds();
    let (no_gps, drawn) = (now - 60 * DAY, now - 20 * DAY);
    reader.answer(drawn, Detail::Route);
    reader.answer(no_gps, Detail::NoGps);
    store_rides(&resources, user_id, tenant, SESSION, &[no_gps, drawn]).await;
    open_window(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert_eq!(
        ask_track_walk(&runtime, user_id, tenant, SESSION).await,
        TrackWalkAsked::Started,
        "bug: the session's track walk was not asked"
    );
    let ended = track_walk_ended(&resources, user_id, tenant).await;
    assert_eq!(
        ended.state,
        BackfillJobState::Walked,
        "bug: the track walk did not end: {ended:?}"
    );
    assert_eq!(
        reader.reads(),
        vec![drawn.to_string(), no_gps.to_string()],
        "bug: a ride no detail read reached is not read for its track, once, newest first"
    );
    match track_of(&resources, user_id, tenant, drawn).await {
        Some(StoredRouteTrack::Drawn { .. }) => {}
        other => panic!("bug: the never-detailed ride did not keep its track: {other:?}"),
    }
    assert_eq!(
        track_of(&resources, user_id, tenant, no_gps).await,
        Some(StoredRouteTrack::Unavailable {
            source: "streams".to_owned(),
            reason: "no_gps".to_owned(),
            expires_at: None,
        }),
        "bug: the never-detailed ride with no GPS did not settle no_gps"
    );
    let rows = rows_under(&resources, user_id, tenant, SESSION).await;
    assert!(
        rows.iter().all(|row| row.segment_read),
        "bug: the track walk's read did not store the ride's efforts: {rows:?}"
    );
}

/// Keep the athlete's `kind` walk as walked down to the epoch, ended at
/// `ended_ms`, as an earlier build's walk that reached its floor is kept.
async fn end_as_walked(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    kind: BackfillJobKind,
    ended_ms: i64,
) {
    let sql = "UPDATE activity_backfill_jobs SET outcome = 'walked', before_ts = 0, \
               ended_at_ms = $2 WHERE user_id = $1 AND kind = $3";
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query(sql)
                .bind(user_id.to_string())
                .bind(ended_ms)
                .bind(kind.as_str())
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            sqlx::query(sql)
                .bind(user_id.to_string())
                .bind(ended_ms)
                .bind(kind.as_str())
                .execute(pg.pool())
                .await
                .unwrap();
        }
    }
}

/// The migration that clears the track walks of the first rule
/// (carnet#1779), per backend, so the test runs the statement a deploy runs.
const CLEAR_TRACK_WALKS_SQLITE: &str =
    include_str!("../../../../migrations/20261124090000_clear_first_rule_track_walks.sql");
#[cfg(feature = "postgresql")]
const CLEAR_TRACK_WALKS_POSTGRES: &str =
    include_str!("../../../../migrations_pg/20261124090000_clear_first_rule_track_walks.sql");

async fn clear_track_walks(resources: &Arc<ServerContext>) {
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::raw_sql(CLEAR_TRACK_WALKS_SQLITE)
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            sqlx::raw_sql(CLEAR_TRACK_WALKS_POSTGRES)
                .execute(pg.pool())
                .await
                .unwrap();
        }
    }
}

/// Record the athlete's `kind` walk through the session as an earlier build
/// left it: walked down to the epoch from `now`.
async fn walked_row(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    kind: BackfillJobKind,
    now: i64,
) {
    let row = ActivityBackfillJobRow {
        id: Uuid::new_v4().to_string(),
        tenant_id: tenant,
        user_id,
        provider: SESSION.to_owned(),
        kind,
        after_ts: None,
        before_ts: Some(now),
        fetch_limit: None,
        conversation_id: None,
        created_at_ms: Timestamp::now().epoch_millis(),
        leased_until_ms: 0,
        attempts: 0,
        lease_token: None,
        progress: JobProgress::default(),
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
        "bug: the earlier build's {kind} fixture row was not recorded"
    );
    end_as_walked(resources, user_id, kind, Timestamp::now().epoch_millis()).await;
}

/// A connection whose track walk walked under the first rule (only rides a
/// detail read reached, carnet#1779) stands until the migration clears its
/// row, and only its track walk row; the next ask then walks it once more:
/// the ride no detail read reached is read, the one the first walk gave a
/// track is not, and the connection is never walked a third time.
#[tokio::test]
async fn a_track_walk_cleared_by_the_migration_reads_the_rides_the_first_rule_passed() {
    let (reader_url, reader) = spawn_session_reader().await;
    let resources = session_server(&reader_url).await;
    let (user_id, tenant) = session_athlete(&resources, "track-walk-cleared").await;
    let (other_user, other_tenant) = session_athlete(&resources, "track-walk-cleared-other").await;
    let now = Timestamp::now().epoch_seconds();
    let (never_read, reached) = (now - 40 * DAY, now - 10 * DAY);
    reader.answer(never_read, Detail::Route);
    reader.answer(reached, Detail::Route);
    store_rides(&resources, user_id, tenant, SESSION, &[never_read]).await;
    // The first walk read this one and kept its track.
    store_walked_rides(&resources, user_id, tenant, &[reached]).await;
    let track = RouteTrack::from_overview(&loop_track()).unwrap();
    settle_track(
        &resources,
        user_id,
        tenant,
        reached,
        &StoredRouteTrack::Drawn {
            source: "streams".to_owned(),
            track_json: serde_json::to_string(&track).unwrap(),
        },
    )
    .await;
    walked_row(&resources, user_id, tenant, BackfillJobKind::TrackWalk, now).await;
    walked_row(
        &resources,
        user_id,
        tenant,
        BackfillJobKind::DetailWalk,
        now,
    )
    .await;
    walked_row(
        &resources,
        other_user,
        other_tenant,
        BackfillJobKind::TrackWalk,
        now,
    )
    .await;
    open_window(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert_eq!(
        ask_track_walk(&runtime, user_id, tenant, SESSION).await,
        TrackWalkAsked::Standing,
        "bug: a walked track walk was asked again before the migration cleared it"
    );
    clear_track_walks(&resources).await;
    for (user, tenant) in [(user_id, tenant), (other_user, other_tenant)] {
        assert_eq!(
            walk_row(&resources, user, tenant, BackfillJobKind::TrackWalk).await,
            None,
            "bug: the migration left a track walk row standing"
        );
    }
    assert!(
        walk_row(&resources, user_id, tenant, BackfillJobKind::DetailWalk)
            .await
            .is_some(),
        "bug: the migration deleted a walk of another kind"
    );

    assert_eq!(
        ask_track_walk(&runtime, user_id, tenant, SESSION).await,
        TrackWalkAsked::Started,
        "bug: a connection whose track walk the migration cleared is not walked again"
    );
    let ended = track_walk_ended(&resources, user_id, tenant).await;
    assert_eq!(
        ended.state,
        BackfillJobState::Walked,
        "bug: the fresh walk did not end: {ended:?}"
    );
    assert_eq!(
        reader.reads(),
        vec![never_read.to_string()],
        "bug: the fresh walk skipped the ride no detail read reached, or re-read one whose track is stored"
    );
    match track_of(&resources, user_id, tenant, never_read).await {
        Some(StoredRouteTrack::Drawn { .. }) => {}
        other => panic!("bug: the never-detailed ride did not keep its track: {other:?}"),
    }

    assert_eq!(
        ask_track_walk(&runtime, user_id, tenant, SESSION).await,
        TrackWalkAsked::Standing,
        "bug: the connection's track walk was asked a third time"
    );
    track_walk_ended(&resources, user_id, tenant).await;
    assert_eq!(
        reader.reads(),
        vec![never_read.to_string()],
        "bug: a ride was read for its track again after the fresh walk"
    );
}

/// A track walk over rides no detail read reached reads nothing once the
/// athlete's walk day is spent: it waits, owed, and reads them in the next
/// day's window.
#[tokio::test]
async fn a_track_walk_of_never_detailed_rides_waits_for_the_walk_day() {
    let (reader_url, reader) = spawn_session_reader().await;
    let resources = session_server(&reader_url).await;
    let (user_id, tenant) = session_athlete(&resources, "track-walk-budget").await;
    let now = Timestamp::now().epoch_seconds();
    let (older, newer) = (now - 30 * DAY, now - 10 * DAY);
    reader.answer(older, Detail::Route);
    reader.answer(newer, Detail::Route);
    store_rides(&resources, user_id, tenant, SESSION, &[older, newer]).await;
    open_window(&resources).await;
    let app = budget_counter_key(oauth_providers::STRAVA, "walk_client");
    resources
        .common
        .repos
        .usage_counters
        .increment_counter(
            PLATFORM_SCOPE,
            PLATFORM_SCOPE,
            &detail_walk_athlete_counter_key(&app, user_id),
            &budget_period(Timestamp::now(), ONE_DAY),
            10_000,
        )
        .await
        .unwrap();

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert_eq!(
        ask_track_walk(&runtime, user_id, tenant, SESSION).await,
        TrackWalkAsked::Started,
        "bug: the session's track walk was not asked"
    );
    let deadline = Instant::now() + WALK_DEADLINE;
    for _ in 0..3 {
        steps_settled_under(
            &resources,
            user_id,
            tenant,
            SESSION,
            BackfillJobKind::TrackWalk,
            deadline,
        )
        .await;
        paused_retries_due(&resources, user_id).await;
        age_job_rows(&resources, user_id).await;
        resume_backfill_jobs(Arc::clone(&runtime)).await.unwrap();
        time::sleep(Duration::from_millis(50)).await;
    }
    steps_settled_under(
        &resources,
        user_id,
        tenant,
        SESSION,
        BackfillJobKind::TrackWalk,
        deadline,
    )
    .await;
    assert_eq!(
        reader.reads(),
        Vec::<String>::new(),
        "bug: the track walk read past the athlete's spent walk day"
    );
    let row = walk_row(&resources, user_id, tenant, BackfillJobKind::TrackWalk)
        .await
        .expect("bug: no track walk row was recorded");
    assert_eq!(
        row.state,
        BackfillJobState::Owed,
        "bug: a spent walk day ended the track walk instead of pausing it: {row:?}"
    );

    let ended = track_walk_ended(&resources, user_id, tenant).await;
    assert_eq!(
        ended.state,
        BackfillJobState::Walked,
        "bug: the track walk did not end once a new day opened: {ended:?}"
    );
    assert_eq!(
        reader.reads(),
        vec![newer.to_string(), older.to_string()],
        "bug: the paused track walk did not read the rides once the day opened"
    );
}

/// The rides of a window a detail walk was asked over are read before any
/// ride is read for its track: a track walk waits while a detail walk is
/// owed through the same session, whatever their starts.
#[tokio::test]
async fn the_rides_no_detail_read_reached_go_before_the_re_reads() {
    let (reader_url, reader) = spawn_session_reader().await;
    let resources = session_server(&reader_url).await;
    let (user_id, tenant) = session_athlete(&resources, "track-walk-order").await;
    let now = Timestamp::now().epoch_seconds();
    let (never_read, walked) = (now - 20 * DAY, now - 5 * DAY);
    reader.answer(never_read, Detail::Route);
    reader.answer(walked, Detail::Route);
    store_rides(&resources, user_id, tenant, SESSION, &[never_read]).await;
    store_walked_rides(&resources, user_id, tenant, &[walked]).await;
    open_window(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert_eq!(
        ask_detail_walk(&runtime, user_id, tenant, SESSION, since(now - 30 * DAY)).await,
        DetailWalkAsked::Started { units_total: 1 },
        "bug: the ride no detail read reached is not owed to the detail walk"
    );
    assert_eq!(
        ask_track_walk(&runtime, user_id, tenant, SESSION).await,
        TrackWalkAsked::Started,
        "bug: the track walk was not asked beside the owed detail walk"
    );
    let ended = track_walk_ended(&resources, user_id, tenant).await;
    assert_eq!(
        ended.state,
        BackfillJobState::Walked,
        "bug: the track walk did not end once the detail walk had: {ended:?}"
    );
    assert_eq!(
        reader.reads(),
        vec![never_read.to_string(), walked.to_string()],
        "bug: a newer ride was read again for its track before the older one no read reached"
    );
}

/// A re-read the service fails stops the walk at that ride, which is read
/// again only up to the attempt cap before the walk ends short; asked again,
/// the walk starts from where it stopped, so the ride it read before the
/// failing one is never read twice.
#[tokio::test]
async fn a_failed_re_read_ends_the_walk_short_and_the_next_ask_resumes_below_it() {
    let (reader_url, reader) = spawn_session_reader().await;
    let resources = session_server(&reader_url).await;
    let (user_id, tenant) = session_athlete(&resources, "track-walk-fails").await;
    let now = Timestamp::now().epoch_seconds();
    let (older, failing, newest) = (now - 20 * DAY, now - 10 * DAY, now - 5 * DAY);
    reader.answer(newest, Detail::NoRoute);
    reader.answer(failing, Detail::Fails);
    reader.answer(older, Detail::Route);
    store_walked_rides(&resources, user_id, tenant, &[older, failing, newest]).await;
    open_window(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert_eq!(
        ask_track_walk(&runtime, user_id, tenant, SESSION).await,
        TrackWalkAsked::Started,
        "bug: the session's track walk was not asked"
    );
    let ended = track_walk_ended(&resources, user_id, tenant).await;
    assert_eq!(
        ended.state,
        BackfillJobState::Incomplete,
        "bug: a ride the service keeps failing did not end the walk short: {ended:?}"
    );
    assert_eq!(
        ended.before_ts,
        Some(newest),
        "bug: the walk did not stop above the failing ride"
    );
    let failed_reads = reader.reads_of(failing);
    assert!(
        (1..=usize::try_from(MAX_BACKFILL_ATTEMPTS).unwrap() + 1).contains(&failed_reads),
        "bug: the failing ride was read {failed_reads} times, past the attempt cap"
    );
    assert_eq!(
        reader.reads_of(older),
        0,
        "bug: the walk read past a ride it failed on"
    );

    reader.answer(failing, Detail::Route);
    assert_eq!(
        ask_track_walk(&runtime, user_id, tenant, SESSION).await,
        TrackWalkAsked::Started,
        "bug: a track walk that ended short is not asked again"
    );
    let ended = track_walk_ended(&resources, user_id, tenant).await;
    assert_eq!(
        ended.state,
        BackfillJobState::Walked,
        "bug: the resumed walk did not end: {ended:?}"
    );
    assert_eq!(
        reader.reads_of(newest),
        1,
        "bug: the resumed walk read again a ride the first one read"
    );
    for start in [failing, older] {
        match track_of(&resources, user_id, tenant, start).await {
            Some(StoredRouteTrack::Drawn { .. }) => {}
            other => panic!("bug: the resumed walk did not keep the ride's track: {other:?}"),
        }
    }
}
