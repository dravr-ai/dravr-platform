// ABOUTME: A session-read detail read that carries the ride's route stores it as the ride's track, trimmed, once
// ABOUTME: An empty route settles no_gps, a missing or failed one settles nothing, and the next places pass forms a place

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Session-read detail tracks (carnet#1679).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// A session-read list row carries no route overview: the ride's GPS line arrives
// only on a detail read, as the session-read detail's `route`. A detail read that
// already happens — the activity view's, the detail walk's — keeps that line
// as the ride's stored track, under the trimming the Home map applies, so the places pass
// can form a place from it. The session reader is a loopback stand-in (a test
// double, per the repo's mock rule) that counts its detail reads, so "no
// extra provider call" is a count.
#![cfg(all(
    feature = "protocol-rest",
    feature = "provider-strava",
    feature = "provider-sciotte",
    feature = "provider-garmin",
    feature = "tools-data",
    feature = "health-sync"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::detail_walk_test::{since, store_rides, walk_ended_under, DAY};
use crate::helpers::sciotte_mock::seed_sciotte_session;
use crate::history_walk_api_test::{open_window, with_window};

use std::collections::HashMap;
use std::slice;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::{to_bytes, Body};
use axum::extract::{Path, State};
use axum::http::{Request, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::models::{
    Activity, ActivityBuilder, ConnectionType, HomeDeclaration, SportType, TenantId,
};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::{BackfillJobState, StoredRouteTrack};
use pierre_fitness_compute::route_track::{RouteTrack, MAP_MAX_POINTS};
use pierre_fitness_compute::routes::haversine_meters_between;
use pierre_fitness_compute::DEFAULT_PRIVACY_RADIUS_METERS;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use pierre_services::athlete_places::places_pass;
use pierre_tool_runtime::activity_backfill::detail_walk::{ask_detail_walk, DetailWalkAsked};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tower::ServiceExt;
use uuid::Uuid;

/// The trailhead every fixture ride starts at, far from any home.
const TRAILHEAD: (f64, f64) = (45.6012, -73.8044);

/// The key a signed-in session files its rides under.
const SESSION: &str = "sciotte";

/// What the session reader stand-in answers for a ride's detail read.
#[derive(Clone)]
enum Detail {
    /// A detail carrying this route.
    Route(Vec<(f64, f64)>),
    /// A detail with no `route` key: the page was read before its map.
    NoRoute,
    /// A `503`: the service is down.
    Fails,
}

/// The session reader stand-in: each ride's answer, and every detail read it saw.
#[derive(Default)]
struct SessionReader {
    answers: Mutex<HashMap<String, Detail>>,
    reads: AtomicUsize,
}

impl SessionReader {
    fn answer(&self, id: &str, detail: Detail) {
        self.answers.lock().unwrap().insert(id.to_owned(), detail);
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

/// `GET /api/activities/{id}`: the ride's detail as [`SessionReader::answer`] set it,
/// started at its id when the id is a unix second (a walk's rides), else two
/// days ago.
async fn detail(
    State(reader): State<Arc<SessionReader>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    reader.reads.fetch_add(1, Ordering::SeqCst);
    let answer = reader
        .answers
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .unwrap_or(Detail::NoRoute);
    let mut body = json!({
        "id": id,
        "name": "Sortie VTT",
        "sport_type": "ride",
        "start_date": id
            .parse::<i64>()
            .ok()
            .and_then(|at| Timestamp::from_epoch_seconds(at).ok())
            .unwrap_or_else(|| Timestamp::now() - Elapsed::days(2))
            .to_rfc3339(),
        "duration_seconds": 5_400,
        "distance_meters": 25_000.0,
        "provider": "strava"
    });
    match answer {
        Detail::Fails => return Err(StatusCode::SERVICE_UNAVAILABLE),
        Detail::NoRoute => {}
        Detail::Route(track) => {
            body["route"] = json!({
                "coordinates": track.iter().map(|&(lat, lon)| json!([lat, lon])).collect::<Vec<_>>()
            });
        }
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

struct Athlete {
    user_id: Uuid,
    tenant: TenantId,
    token: String,
}

/// An athlete holding a live signed-in session for Strava.
async fn sciotte_athlete(resources: &Arc<ServerContext>, label: &str) -> Athlete {
    let email = format!("{label}-{}@example.com", Uuid::new_v4());
    let (user_id, user, tenant) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .unwrap();
    let token = common::generate_test_token(resources, &user).await;
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, SESSION, &ConnectionType::Manual, None)
        .await
        .unwrap();
    seed_sciotte_session(resources, user_id, tenant).await;
    Athlete {
        user_id,
        tenant,
        token,
    }
}

/// A mountain-bike loop north from the trailhead, ~10 km, `phase` shifting
/// its weave so no two rides share a line.
fn loop_track(phase: f64) -> Vec<(f64, f64)> {
    (0..900)
        .map(|i| {
            let step = f64::from(i);
            (
                0.0001f64.mul_add(step, TRAILHEAD.0),
                0.0003f64.mul_add(step.mul_add(0.17, phase).sin(), TRAILHEAD.1),
            )
        })
        .collect()
}

/// A ride as the session-read list read stores it: a raw start, no route overview.
fn listed_ride(id: &str, started: Timestamp) -> Activity {
    ActivityBuilder::new(
        id,
        "Sortie VTT",
        SportType::MountainBike,
        started,
        5_400,
        "strava",
    )
    .distance_meters(25_000.0)
    .start_latitude(TRAILHEAD.0)
    .start_longitude(TRAILHEAD.1)
    .build()
}

async fn cache_rides(resources: &Arc<ServerContext>, athlete: &Athlete, rides: &[Activity]) {
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(athlete.user_id, &athlete.tenant, SESSION, rides)
        .await
        .unwrap();
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

/// Open the ride's activity view: its one detail read.
async fn open_view(resources: &Arc<ServerContext>, athlete: &Athlete, id: &str) {
    let (status, body) = get_json(
        resources,
        &athlete.token,
        &format!("/api/me/activities/strava/{id}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "bug: the view failed: {body}");
}

async fn stored_track(
    resources: &Arc<ServerContext>,
    tenant: &TenantId,
    user_id: Uuid,
    id: &str,
) -> Option<StoredRouteTrack> {
    resources
        .common
        .repos
        .activity_route_tracks
        .get_route_track(tenant, user_id, SESSION, id)
        .await
        .unwrap()
}

/// The decoded drawn track stored for the athlete's ride.
async fn drawn(resources: &Arc<ServerContext>, athlete: &Athlete, id: &str) -> RouteTrack {
    match stored_track(resources, &athlete.tenant, athlete.user_id, id).await {
        Some(StoredRouteTrack::Drawn { source, track_json }) => {
            assert_eq!(
                source, "streams",
                "bug: a detail read's route is stored as a streams read"
            );
            serde_json::from_str(&track_json).unwrap()
        }
        other => panic!("bug: the ride's route was not stored as a drawn track: {other:?}"),
    }
}

/// Move every stored detail's recheck past, so the next view reads again.
async fn pass_detail_rechecks(resources: &Arc<ServerContext>, user_id: Uuid) {
    const SQL: &str = "UPDATE cached_activities SET detail_recheck_at = $1 \
                       WHERE user_id = $2 AND detail_recheck_at IS NOT NULL";
    let past = Timestamp::now() - Elapsed::minutes(1);
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query(SQL)
                .bind(past)
                .bind(user_id.to_string())
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => {
            sqlx::query(SQL)
                .bind(past)
                .bind(user_id.to_string())
                .execute(postgres.pool())
                .await
                .unwrap();
        }
    }
}

fn metres(a: (f64, f64), b: (f64, f64)) -> f64 {
    haversine_meters_between(a.0, a.1, b.0, b.1)
}

#[tokio::test]
async fn a_detail_read_carrying_a_route_stores_the_rides_trimmed_track_with_no_extra_read() {
    let (reader_url, reader) = spawn_session_reader().await;
    let resources = common::create_test_server_resources_with_config(
        common::test_server_config_with_sciotte(&reader_url),
    )
    .await
    .unwrap();
    let athlete = sciotte_athlete(&resources, "sciotte-track").await;
    let recorded = loop_track(0.0);
    reader.answer("t1", Detail::Route(recorded.clone()));
    cache_rides(
        &resources,
        &athlete,
        &[listed_ride("t1", Timestamp::now() - Elapsed::days(2))],
    )
    .await;

    open_view(&resources, &athlete, "t1").await;
    assert_eq!(reader.reads(), 1, "bug: the view reads the detail once");

    let track = drawn(&resources, &athlete, "t1").await;
    let first = track.first_point().unwrap();
    let last = track.last_point().unwrap();
    assert!(
        metres(first, recorded[0]) >= DEFAULT_PRIVACY_RADIUS_METERS,
        "bug: the stored track starts {:.0} m from the recorded start, inside the privacy radius",
        metres(first, recorded[0])
    );
    assert!(
        metres(last, recorded[recorded.len() - 1]) >= DEFAULT_PRIVACY_RADIUS_METERS,
        "bug: the stored track ends inside the privacy radius of the recorded end"
    );
    assert!(
        track.coordinates.len() > 2 && track.coordinates.len() <= MAP_MAX_POINTS,
        "bug: the stored track is not simplified to a map's points: {}",
        track.coordinates.len()
    );

    // The Home map answers from what the detail read stored: no read.
    let (status, route) = get_json(
        &resources,
        &athlete.token,
        "/api/me/activities/strava/t1/route",
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: the route read failed: {route}"
    );
    assert!(
        route["route"].is_object(),
        "bug: the Home map does not draw the stored track: {route}"
    );
    assert_eq!(
        reader.reads(),
        1,
        "bug: the Home map read the provider again for a track the detail read stored"
    );
}

#[tokio::test]
async fn the_stored_track_is_the_athletes_own_in_their_own_tenant() {
    let (reader_url, reader) = spawn_session_reader().await;
    let resources = common::create_test_server_resources_with_config(
        common::test_server_config_with_sciotte(&reader_url),
    )
    .await
    .unwrap();
    let athlete = sciotte_athlete(&resources, "sciotte-track-scope").await;
    let other = sciotte_athlete(&resources, "sciotte-track-other").await;
    reader.answer("t2", Detail::Route(loop_track(0.4)));
    let ride = listed_ride("t2", Timestamp::now() - Elapsed::days(2));
    cache_rides(&resources, &athlete, slice_of(&ride)).await;
    cache_rides(&resources, &other, slice_of(&ride)).await;

    open_view(&resources, &athlete, "t2").await;
    drawn(&resources, &athlete, "t2").await;
    assert_eq!(
        stored_track(&resources, &other.tenant, other.user_id, "t2").await,
        None,
        "bug: another athlete holding the same ride id got this athlete's track"
    );
    assert_eq!(
        stored_track(&resources, &other.tenant, athlete.user_id, "t2").await,
        None,
        "bug: the athlete's track leaked into another tenant"
    );
    assert_eq!(
        stored_track(&resources, &athlete.tenant, other.user_id, "t2").await,
        None,
        "bug: the athlete's track was filed under another user"
    );
}

fn slice_of(ride: &Activity) -> &[Activity] {
    slice::from_ref(ride)
}

#[tokio::test]
async fn an_empty_route_settles_no_gps_and_only_a_drawn_track_replaces_it() {
    let (reader_url, reader) = spawn_session_reader().await;
    let resources = common::create_test_server_resources_with_config(
        common::test_server_config_with_sciotte(&reader_url),
    )
    .await
    .unwrap();
    let athlete = sciotte_athlete(&resources, "sciotte-track-nogps").await;
    reader.answer("indoor", Detail::Route(Vec::new()));
    reader.answer("drawn", Detail::Route(loop_track(0.8)));
    cache_rides(
        &resources,
        &athlete,
        &[
            listed_ride("indoor", Timestamp::now() - Elapsed::days(3)),
            listed_ride("drawn", Timestamp::now() - Elapsed::days(2)),
        ],
    )
    .await;

    open_view(&resources, &athlete, "indoor").await;
    assert_eq!(
        stored_track(&resources, &athlete.tenant, athlete.user_id, "indoor").await,
        Some(StoredRouteTrack::Unavailable {
            source: "streams".to_owned(),
            reason: "no_gps".to_owned(),
            expires_at: None,
        }),
        "bug: a read whose route holds no coordinates settles no_gps, for good"
    );

    // A later read that draws the ride's line proves the empty one lost it.
    reader.answer("indoor", Detail::Route(loop_track(0.2)));
    pass_detail_rechecks(&resources, athlete.user_id).await;
    let before = reader.reads();
    open_view(&resources, &athlete, "indoor").await;
    assert_eq!(
        reader.reads(),
        before + 1,
        "bug: the fixture did not make the view read the ride's detail again"
    );
    drawn(&resources, &athlete, "indoor").await;

    open_view(&resources, &athlete, "drawn").await;
    drawn(&resources, &athlete, "drawn").await;
    // A later read of the same ride whose route came back empty is a read
    // glitch, never a reason to drop the line a read already drew.
    reader.answer("drawn", Detail::Route(Vec::new()));
    pass_detail_rechecks(&resources, athlete.user_id).await;
    let before = reader.reads();
    open_view(&resources, &athlete, "drawn").await;
    assert_eq!(
        reader.reads(),
        before + 1,
        "bug: the fixture did not make the view read the ride's detail again"
    );
    drawn(&resources, &athlete, "drawn").await;
}

#[tokio::test]
async fn a_read_with_no_route_or_a_failed_read_settles_nothing() {
    let (reader_url, reader) = spawn_session_reader().await;
    let resources = common::create_test_server_resources_with_config(
        common::test_server_config_with_sciotte(&reader_url),
    )
    .await
    .unwrap();
    let athlete = sciotte_athlete(&resources, "sciotte-track-unread").await;
    reader.answer("unrendered", Detail::NoRoute);
    reader.answer("down", Detail::Fails);
    cache_rides(
        &resources,
        &athlete,
        &[
            listed_ride("unrendered", Timestamp::now() - Elapsed::days(3)),
            listed_ride("down", Timestamp::now() - Elapsed::days(2)),
        ],
    )
    .await;

    open_view(&resources, &athlete, "unrendered").await;
    open_view(&resources, &athlete, "down").await;
    assert!(
        reader.reads() >= 2,
        "bug: the fixture's views did not read the provider"
    );
    for id in ["unrendered", "down"] {
        assert_eq!(
            stored_track(&resources, &athlete.tenant, athlete.user_id, id).await,
            None,
            "bug: a read that carried no route ({id}) stored a route answer; only a read \
             whose route holds no coordinates settles no_gps"
        );
    }
}

#[tokio::test]
async fn the_next_places_pass_forms_a_place_from_the_stored_tracks() {
    let (reader_url, reader) = spawn_session_reader().await;
    let resources = common::create_test_server_resources_with_config(
        common::test_server_config_with_sciotte(&reader_url),
    )
    .await
    .unwrap();
    let athlete = sciotte_athlete(&resources, "sciotte-track-place").await;
    let repos = &resources.common.repos;
    let ids = ["p1", "p2", "p3"];
    let rides: Vec<Activity> = ids
        .iter()
        .zip(1_i64..)
        .map(|(id, days)| listed_ride(id, Timestamp::now() - Elapsed::days(days * 2)))
        .collect();
    for (id, phase) in ids.iter().zip([0.0, 0.5, 1.0]) {
        reader.answer(id, Detail::Route(loop_track(phase)));
    }
    cache_rides(&resources, &athlete, &rides).await;
    // Home is not the trailhead: no start is excluded as home.
    let declared = Timestamp::now() - Elapsed::hours(3);
    repos
        .athlete_places
        .store_home_declaration(
            &athlete.tenant,
            athlete.user_id,
            HomeDeclaration::Elsewhere,
            declared,
        )
        .await
        .unwrap();

    // Raw starts alone never create a place.
    let first_pass = Timestamp::now() - Elapsed::hours(2);
    places_pass(repos, first_pass, 50).await.unwrap();
    assert!(
        repos
            .athlete_places
            .list(&athlete.tenant, athlete.user_id)
            .await
            .unwrap()
            .is_empty(),
        "bug: raw starts formed a place"
    );

    for id in ids {
        open_view(&resources, &athlete, id).await;
    }
    let reads = reader.reads();
    assert_eq!(reads, ids.len(), "bug: each view reads its ride once");

    // Within the day's backstop: only the stored tracks make the athlete due.
    let report = places_pass(repos, Timestamp::now(), 50).await.unwrap();
    assert_eq!(
        report.users, 1,
        "bug: a newly stored track did not make the athlete due a places pass"
    );
    let places = repos
        .athlete_places
        .list(&athlete.tenant, athlete.user_id)
        .await
        .unwrap();
    assert_eq!(
        places.len(),
        1,
        "bug: the three rides' stored tracks formed no place at their trailhead: {places:?}"
    );
    assert_eq!(
        places[0].activity_count, 3,
        "bug: the place does not hold the three rides"
    );
    assert_eq!(
        reader.reads(),
        reads,
        "bug: the places pass asked the provider for a track"
    );
}

/// The detail walk through the athlete's session keeps each ride's route as
/// it reads the ride for its efforts: one read per ride, keyed under the
/// session the walk reads.
#[tokio::test]
async fn a_session_detail_walk_stores_each_rides_route_with_its_one_read() {
    let (reader_url, reader) = spawn_session_reader().await;
    let mut config = common::test_server_config_with_sciotte(&reader_url);
    common::set_server_app(&mut config, "strava", "walk_client", "walk_secret");
    let resources = common::create_test_server_resources_with_config(config)
        .await
        .unwrap();
    let resources = with_window(&resources, 400);
    let athlete = sciotte_athlete(&resources, "sciotte-track-walk").await;
    let now = Timestamp::now().epoch_seconds();
    let (outdoor, indoor, unrendered) = (now - 10 * DAY, now - 8 * DAY, now - 6 * DAY);
    reader.answer(&outdoor.to_string(), Detail::Route(loop_track(0.3)));
    reader.answer(&indoor.to_string(), Detail::Route(Vec::new()));
    reader.answer(&unrendered.to_string(), Detail::NoRoute);
    store_rides(
        &resources,
        athlete.user_id,
        athlete.tenant,
        SESSION,
        &[outdoor, indoor, unrendered],
    )
    .await;
    open_window(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let asked = ask_detail_walk(
        &runtime,
        athlete.user_id,
        athlete.tenant,
        SESSION,
        since(now - 30 * DAY),
    )
    .await;
    assert_eq!(
        asked,
        DetailWalkAsked::Started { units_total: 3 },
        "bug: the fixture's three rides are not owed to the walk"
    );
    let ended = walk_ended_under(&resources, athlete.user_id, athlete.tenant, SESSION).await;
    assert_eq!(
        ended.state,
        BackfillJobState::Walked,
        "bug: the session walk did not end at its floor: {ended:?}"
    );
    assert_eq!(
        reader.reads(),
        3,
        "bug: the walk read a ride more than once to keep its route"
    );

    drawn(&resources, &athlete, &outdoor.to_string()).await;
    assert_eq!(
        stored_track(
            &resources,
            &athlete.tenant,
            athlete.user_id,
            &indoor.to_string()
        )
        .await,
        Some(StoredRouteTrack::Unavailable {
            source: "streams".to_owned(),
            reason: "no_gps".to_owned(),
            expires_at: None,
        }),
        "bug: the walk's read of an empty route did not settle no_gps"
    );
    assert_eq!(
        stored_track(
            &resources,
            &athlete.tenant,
            athlete.user_id,
            &unrendered.to_string()
        )
        .await,
        None,
        "bug: the walk's read that carried no route stored a route answer"
    );
}
