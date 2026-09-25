// ABOUTME: GET /api/me/activities/{provider}/{activity_id}/route — a cached activity's trimmed route, read once and stored
// ABOUTME: The route overview costs no provider call; streams cost one; no_gps and too_short are answers, not errors

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Athlete Home activity-route suite.
//!
//! The Strava-backed cases point the real Strava provider at a local mock
//! (`PIERRE_STRAVA_API_BASE_URL`, the registry seam) that counts every detail
//! and streams read, so "the second read does no provider fetch" is a count,
//! not a hope. Geometry assertions are on real coordinates: a handler that
//! drew the untrimmed track, an empty line or another tenant's route fails.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
#![cfg(all(feature = "provider-strava", feature = "protocol-rest"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{Duration, Utc};
use pierre_core::models::{
    Activity, ActivityBuilder, ConnectionType, SportType, Tenant, TenantId, User, UserOAuthToken,
};
use pierre_database::repositories::StoredRouteTrack;
use pierre_fitness_compute::routes::haversine_meters_between;
use pierre_fitness_compute::{encode_polyline, DEFAULT_PRIVACY_RADIUS_METERS};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use pierre_mcp_server::services::activity_route::HOME_ROUTE_MAX_POINTS;
use pierre_routes_auth::OAuthService;
use pierre_services::provider_revocation::DisconnectReason;
use serde_json::{json, Value};
use serial_test::serial;
use tokio::net::TcpListener;
use tower::ServiceExt;
use uuid::Uuid;

/// Parc La Fontaine, Montréal — where every fixture track starts.
const HOME: (f64, f64) = (45.5259, -73.5697);

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
async fn second_tenant(resources: &Arc<ServerContext>, athlete: &Athlete) -> (TenantId, String) {
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
    let token = resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &athlete.user,
            &resources.auth.jwks_manager,
            Some(tenant.to_string()),
        )
        .unwrap();
    (tenant, token)
}

async fn route_of(
    resources: &Arc<ServerContext>,
    token: &str,
    provider: &str,
    activity_id: &str,
) -> (StatusCode, Value) {
    let response = athlete_home_routes()
        .with_state(Arc::clone(resources))
        .oneshot(
            Request::builder()
                .uri(format!("/api/me/activities/{provider}/{activity_id}/route"))
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

/// A northbound track weaving east and west, `count` samples ~11 m apart.
fn weaving_track(count: usize, phase: f64) -> Vec<(f64, f64)> {
    (0..count)
        .map(|i| {
            let step = i as f64;
            (
                0.0001f64.mul_add(step, HOME.0),
                0.0005f64.mul_add(step.mul_add(0.21, phase).sin(), HOME.1),
            )
        })
        .collect()
}

fn metres(a: (f64, f64), b: (f64, f64)) -> f64 {
    haversine_meters_between(a.0, a.1, b.0, b.1)
}

fn coordinates(route: &Value) -> Vec<(f64, f64)> {
    route["coordinates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| (pair[0].as_f64().unwrap(), pair[1].as_f64().unwrap()))
        .collect()
}

/// An outdoor run whose list payload carried a route overview.
fn run_with_overview(id: &str, provider: &str, track: &[(f64, f64)]) -> Activity {
    ActivityBuilder::new(
        id,
        "Hill loop",
        SportType::Run,
        Utc::now() - Duration::days(1),
        3_000,
        provider,
    )
    .start_latitude(track[0].0)
    .start_longitude(track[0].1)
    .summary_polyline(encode_polyline(track))
    .build()
}

/// An outdoor run cached before overviews were carried: a start, no line.
fn run_without_overview(id: &str) -> Activity {
    ActivityBuilder::new(
        id,
        "Long ride",
        SportType::Ride,
        Utc::now() - Duration::days(2),
        9_000,
        "strava",
    )
    .start_latitude(HOME.0)
    .start_longitude(HOME.1)
    .build()
}

async fn cache(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    provider: &str,
    rows: &[Activity],
) {
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(user_id, &tenant, provider, rows)
        .await
        .unwrap();
}

async fn stored(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    provider: &str,
    activity_id: &str,
) -> Option<StoredRouteTrack> {
    resources
        .common
        .repos
        .activity_route_tracks
        .get_route_track(&athlete.tenant, athlete.user_id, provider, activity_id)
        .await
        .unwrap()
}

// ---------------------------------------------------------------------------
// A Strava-shaped mock serving one activity's detail and streams
// ---------------------------------------------------------------------------

/// Environment set for the duration of one test and removed after it.
struct EnvGuard {
    keys: Vec<&'static str>,
}

impl EnvGuard {
    fn set(vars: &[(&'static str, String)]) -> Self {
        for (key, value) in vars {
            env::set_var(key, value);
        }
        Self {
            keys: vars.iter().map(|(key, _)| *key).collect(),
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for key in &self.keys {
            env::remove_var(key);
        }
    }
}

/// How many times the mock was asked for a detail and for streams.
#[derive(Default)]
struct Hits {
    detail: AtomicUsize,
    streams: AtomicUsize,
}

impl Hits {
    fn counts(&self) -> (usize, usize) {
        (
            self.detail.load(Ordering::SeqCst),
            self.streams.load(Ordering::SeqCst),
        )
    }
}

/// A mock answering `/activities/{id}` and `/activities/{id}/streams` with
/// `streams` as the keyed stream set.
async fn mock_strava(streams: Value) -> (String, Arc<Hits>) {
    let hits = Arc::new(Hits::default());
    let detail_hits = Arc::clone(&hits);
    let stream_hits = Arc::clone(&hits);
    let app = Router::new()
        .route(
            "/activities/{id}",
            get(move || {
                let hits = Arc::clone(&detail_hits);
                async move {
                    hits.detail.fetch_add(1, Ordering::SeqCst);
                    Json(json!({
                        "id": 55_001,
                        "name": "Long ride",
                        "type": "Ride",
                        "sport_type": "Ride",
                        "start_date": (Utc::now() - Duration::days(2)).to_rfc3339(),
                        "elapsed_time": 9_000,
                        "distance": 60_000.0
                    }))
                }
            }),
        )
        .route(
            "/activities/{id}/streams",
            get(move || {
                let hits = Arc::clone(&stream_hits);
                let streams = streams.clone();
                async move {
                    hits.streams.fetch_add(1, Ordering::SeqCst);
                    Json(streams)
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

/// A keyed stream set for `track`, climbing 0.3 m a sample, or with no GPS
/// channel at all when `track` is empty (a trainer ride).
fn streams_for(track: &[(f64, f64)]) -> Value {
    let samples = if track.is_empty() { 600 } else { track.len() };
    let mut streams = json!({
        "time": { "data": (0..samples).collect::<Vec<_>>() },
        "heartrate": { "data": vec![140; samples] },
        "altitude": { "data": (0..samples).map(|i| 0.3f64.mul_add(i as f64, 20.0)).collect::<Vec<_>>() },
    });
    if !track.is_empty() {
        streams["latlng"] = json!({
            "data": track.iter().map(|&(lat, lon)| json!([lat, lon])).collect::<Vec<_>>()
        });
    }
    streams
}

/// Point the Strava provider at `api_base` for the life of the guard. The
/// registry reads the override when the server context is built, so the
/// guard must be alive before `create_test_server_resources`.
fn strava_at(api_base: String) -> EnvGuard {
    EnvGuard::set(&[
        ("PIERRE_STRAVA_API_BASE_URL", api_base),
        ("STRAVA_CLIENT_ID", "test_client".to_owned()),
        ("STRAVA_CLIENT_SECRET", "test_secret".to_owned()),
    ])
}

async fn link_strava(resources: &Arc<ServerContext>, athlete: &Athlete) {
    let repos = &resources.common.repos;
    repos
        .provider_connections
        .register_connection(
            athlete.user_id,
            athlete.tenant,
            "strava",
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
    let now = Utc::now();
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id: athlete.user_id,
            tenant_id: athlete.tenant.to_string(),
            provider: "strava".to_owned(),
            // >= 40 chars and not "at_"-prefixed: the provider's token validation.
            access_token: "strava_access_token_0123456789abcdef0123456789".to_owned(),
            refresh_token: Some("strava_refresh".to_owned()),
            token_type: "Bearer".to_owned(),
            expires_at: Some(now + Duration::hours(6)),
            scope: Some("activity:read_all".to_owned()),
            provider_user_id: Some("4242".to_owned()),
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------
// The route overview: no provider call at all
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn a_route_overview_draws_the_map_without_a_provider_call() {
    let (api_base, hits) = mock_strava(streams_for(&weaving_track(900, 0.0))).await;
    let _env = strava_at(api_base);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "route-overview").await;
    link_strava(&resources, &athlete).await;
    let track = weaving_track(300, 0.0);
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "strava",
        &[run_with_overview("o1", "strava", &track)],
    )
    .await;

    let (status, body) = route_of(&resources, &athlete.token, "strava", "o1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["reason", "route"], "both keys are always present");
    assert!(body["reason"].is_null());
    let route = &body["route"];
    assert_eq!(route["source_tool"], "strava");
    assert_eq!(route["title"], "Hill loop");
    assert_eq!(route["climbs"], json!([]));
    assert!(
        route["elevation_meters"].is_null(),
        "an overview has no vertical"
    );
    assert!(route["distances_meters"].is_null());
    let drawn = coordinates(route);
    assert!(drawn.len() >= 2 && drawn.len() < track.len());
    assert!(metres(drawn[0], track[0]) >= DEFAULT_PRIVACY_RADIUS_METERS - 1.0);
    assert!(
        metres(*drawn.last().unwrap(), *track.last().unwrap())
            >= DEFAULT_PRIVACY_RADIUS_METERS - 1.0
    );
    let bounds = &route["bounds"];
    for &(lat, lon) in &drawn {
        assert!(bounds["min_latitude"].as_f64().unwrap() <= lat);
        assert!(lat <= bounds["max_latitude"].as_f64().unwrap());
        assert!(bounds["min_longitude"].as_f64().unwrap() <= lon);
        assert!(lon <= bounds["max_longitude"].as_f64().unwrap());
    }

    assert_eq!(hits.counts(), (0, 0), "the overview costs no provider read");
    assert!(matches!(
        stored(&resources, &athlete, "strava", "o1").await,
        Some(StoredRouteTrack::Drawn { ref source, .. }) if source == "summary_polyline"
    ));
}

// ---------------------------------------------------------------------------
// The streams: one read, then stored
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn without_an_overview_the_streams_draw_it_once_and_the_next_read_is_stored() {
    let recorded = weaving_track(900, 0.0);
    let (api_base, hits) = mock_strava(streams_for(&recorded)).await;
    let _env = strava_at(api_base);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "route-streams").await;
    link_strava(&resources, &athlete).await;
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "strava",
        &[run_without_overview("55001")],
    )
    .await;

    let (status, first) = route_of(&resources, &athlete.token, "strava", "55001").await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(hits.counts(), (1, 1), "one detail read, one streams read");
    let route = &first["route"];
    let drawn = coordinates(route);
    assert!(drawn.len() <= HOME_ROUTE_MAX_POINTS);
    assert!(drawn.len() > 2, "a weaving ride keeps its corners");
    assert!(metres(drawn[0], recorded[0]) >= DEFAULT_PRIVACY_RADIUS_METERS);
    let elevations = route["elevation_meters"].as_array().unwrap();
    let distances = route["distances_meters"].as_array().unwrap();
    assert_eq!(elevations.len(), drawn.len());
    assert_eq!(distances.len(), drawn.len());
    assert!(
        distances[0].as_f64().unwrap() >= DEFAULT_PRIVACY_RADIUS_METERS,
        "the line picks the ride up where the trim leaves it"
    );
    assert_eq!(route["title"], "Long ride");
    assert!(matches!(
        stored(&resources, &athlete, "strava", "55001").await,
        Some(StoredRouteTrack::Drawn { ref source, .. }) if source == "streams"
    ));

    let (status, second) = route_of(&resources, &athlete.token, "strava", "55001").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        second, first,
        "the stored read answers exactly what the first did"
    );
    assert_eq!(
        hits.counts(),
        (1, 1),
        "the second read costs no provider call"
    );
}

#[tokio::test]
#[serial]
async fn a_trainer_ride_answers_no_gps_and_is_not_read_again() {
    let (api_base, hits) = mock_strava(streams_for(&[])).await;
    let _env = strava_at(api_base);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "route-indoor").await;
    link_strava(&resources, &athlete).await;
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "strava",
        &[run_without_overview("55002")],
    )
    .await;

    for _ in 0..2 {
        let (status, body) = route_of(&resources, &athlete.token, "strava", "55002").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body, json!({ "route": null, "reason": "no_gps" }));
    }
    assert_eq!(hits.counts(), (1, 1), "the answer is stored, not re-read");
    assert_eq!(
        stored(&resources, &athlete, "strava", "55002").await,
        Some(StoredRouteTrack::Unavailable {
            source: "streams".to_owned(),
            reason: "no_gps".to_owned(),
        })
    );
}

#[tokio::test]
#[serial]
async fn a_ride_that_never_leaves_the_doorstep_answers_too_short() {
    let (api_base, hits) = mock_strava(streams_for(&weaving_track(12, 0.0))).await;
    let _env = strava_at(api_base);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "route-short").await;
    link_strava(&resources, &athlete).await;
    // The overview is too short to settle it, so the streams decide.
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "strava",
        &[run_with_overview(
            "55003",
            "strava",
            &weaving_track(12, 0.0),
        )],
    )
    .await;

    let (status, body) = route_of(&resources, &athlete.token, "strava", "55003").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "route": null, "reason": "too_short" }));
    assert_eq!(hits.counts(), (1, 1));
}

// ---------------------------------------------------------------------------
// Ownership and tenancy
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_activity_the_caller_does_not_hold_here_is_not_found() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "route-owner").await;
    let stranger = seed_athlete(&resources, "route-stranger").await;
    let (_, elsewhere_token) = second_tenant(&resources, &athlete).await;
    let track = weaving_track(300, 0.0);
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "strava",
        &[run_with_overview("mine", "strava", &track)],
    )
    .await;

    let (status, _) = route_of(&resources, &athlete.token, "strava", "missing").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = route_of(&resources, &athlete.token, "whoop", "mine").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the id belongs to another provider"
    );
    let (status, _) = route_of(&resources, &stranger.token, "strava", "mine").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "another athlete's activity");
    let (status, _) = route_of(&resources, &elsewhere_token, "strava", "mine").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "the athlete's other tenant");
    let (status, _) = route_of(&resources, &athlete.token, "strava", "mine").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_stored_route_is_read_only_in_its_own_tenant() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "route-tenancy").await;
    let (elsewhere, elsewhere_token) = second_tenant(&resources, &athlete).await;
    let here = weaving_track(300, 0.0);
    let there = weaving_track(300, 1.3);
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "strava",
        &[run_with_overview("same-id", "strava", &here)],
    )
    .await;
    cache(
        &resources,
        athlete.user_id,
        elsewhere,
        "strava",
        &[run_with_overview("same-id", "strava", &there)],
    )
    .await;

    let (_, home_body) = route_of(&resources, &athlete.token, "strava", "same-id").await;
    let (_, elsewhere_body) = route_of(&resources, &elsewhere_token, "strava", "same-id").await;
    let home_line = coordinates(&home_body["route"]);
    let elsewhere_line = coordinates(&elsewhere_body["route"]);
    assert_ne!(home_line, elsewhere_line, "each tenant draws its own row");
    // Read again: each tenant's stored route answers for that tenant only.
    let (_, again) = route_of(&resources, &elsewhere_token, "strava", "same-id").await;
    assert_eq!(coordinates(&again["route"]), elsewhere_line);
}

#[tokio::test]
async fn a_mirror_backend_activity_is_reached_through_the_provider_it_mirrors() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "route-mirror").await;
    let track = weaving_track(300, 0.0);
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "sciotte",
        &[run_with_overview("m1", "sciotte", &track)],
    )
    .await;

    let (status, body) = route_of(&resources, &athlete.token, "strava", "m1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["route"]["source_tool"], "strava",
        "the mirror is never named to the athlete"
    );
    assert!(
        stored(&resources, &athlete, "sciotte", "m1")
            .await
            .is_some(),
        "the route is filed under the row's own key, so the mirror's purge takes it"
    );
}

#[tokio::test]
async fn disconnecting_a_provider_deletes_the_routes_it_drew() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "route-purge").await;
    let track = weaving_track(300, 0.0);
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "strava",
        &[run_with_overview("s1", "strava", &track)],
    )
    .await;
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "whoop",
        &[run_with_overview("w1", "whoop", &track)],
    )
    .await;
    for (provider, id) in [("strava", "s1"), ("whoop", "w1")] {
        let (status, _) = route_of(&resources, &athlete.token, provider, id).await;
        assert_eq!(status, StatusCode::OK);
        assert!(stored(&resources, &athlete, provider, id).await.is_some());
    }

    // No Strava token is stored, so the revocation step sends nothing
    // upstream; the purge is what is under test.
    OAuthService::new(
        resources.data(),
        Arc::new((*resources.common.config).clone()),
    )
    .disconnect_provider(
        athlete.user_id,
        "strava",
        Some(athlete.tenant.as_uuid()),
        DisconnectReason::Athlete,
    )
    .await
    .unwrap();

    assert!(stored(&resources, &athlete, "strava", "s1").await.is_none());
    assert!(
        stored(&resources, &athlete, "whoop", "w1").await.is_some(),
        "another provider's routes stay"
    );
    let (status, _) = route_of(&resources, &athlete.token, "strava", "s1").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "the activity went with it");
}
