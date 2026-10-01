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
//! The mirror-backend cases point the sciotte provider at a local stand-in
//! for the scraper service (`DRAVR_SCIOTTE_REMOTE_URL`) that counts its
//! detail reads the same way.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
#![cfg(all(feature = "provider-strava", feature = "protocol-rest"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
#[path = "helpers/sciotte_mock.rs"]
mod sciotte_mock;

use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration as StdDuration;

use axum::body::{to_bytes, Body};
use axum::extract::Path;
use axum::http::StatusCode as HttpStatus;
use axum::http::{Request, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use futures_util::future::join_all;
use pierre_core::models::{
    Activity, ActivityBuilder, ConnectionType, SportType, Tenant, TenantId, User, UserOAuthToken,
};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::StoredRouteTrack;
use pierre_fitness_compute::routes::haversine_meters_between;
use pierre_fitness_compute::{encode_polyline, DEFAULT_PRIVACY_RADIUS_METERS};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use pierre_mcp_server::services::activity_detail::EMPTY_DETAIL_RECHECK_MINUTES;
use pierre_mcp_server::services::activity_route::{
    HOME_ROUTE_MAX_POINTS, UNREAD_ROUTE_RECHECK_MINUTES,
};
use pierre_routes_auth::OAuthService;
use pierre_services::provider_revocation::DisconnectReason;
use serde_json::{json, Value};
use serial_test::serial;
use tokio::net::TcpListener;
use tokio::time::sleep;
use tower::ServiceExt;
use uuid::Uuid;

use crate::sciotte_mock::seed_sciotte_session;

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

async fn route_of(
    resources: &Arc<ServerContext>,
    token: &str,
    provider: &str,
    activity_id: &str,
) -> (StatusCode, Value) {
    get_json(
        resources,
        token,
        &format!("/api/me/activities/{provider}/{activity_id}/route"),
    )
    .await
}

/// What the Home list says of one activity's GPS.
async fn listed_has_gps(resources: &Arc<ServerContext>, token: &str, activity_id: &str) -> bool {
    let (status, body) = get_json(resources, token, "/api/me/activities/recent?limit=20").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["activities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == activity_id)
        .unwrap_or_else(|| panic!("{activity_id} is not on the list: {body}"))["has_gps"]
        .as_bool()
        .unwrap()
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

/// A ride as a list that carries neither a start position nor a route
/// overview caches it, whether or not the ride recorded GPS.
fn ride_without_position(id: &str) -> Activity {
    ActivityBuilder::new(
        id,
        "Long ride",
        SportType::Ride,
        Utc::now() - Duration::days(2),
        9_000,
        "strava",
    )
    .distance_meters(60_000.0)
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

/// Move every stored read of the athlete's that carries an expiry to a minute
/// ago, as if the day it stood for had passed. No repository writes a past
/// expiry for a read it has just made, so the fixture writes it in SQL; a
/// read stored without an expiry is left as it is.
async fn expire_route_reads(resources: &ServerContext, user_id: Uuid) {
    const SQL: &str = "UPDATE activity_route_tracks SET expires_at = $1 \
                       WHERE user_id = $2 AND expires_at IS NOT NULL";
    let past = Utc::now() - Duration::minutes(1);
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

/// Run `sql` against the test backend, binding `at` then the user's id.
async fn exec_for_user(resources: &ServerContext, sql: &str, at: DateTime<Utc>, user_id: Uuid) {
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query(sql)
                .bind(at)
                .bind(user_id.to_string())
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => {
            sqlx::query(sql)
                .bind(at)
                .bind(user_id.to_string())
                .execute(postgres.pool())
                .await
                .unwrap();
        }
    }
}

/// Move every stored detail read of the user's that has a recheck instant
/// past it, as the clock would after [`EMPTY_DETAIL_RECHECK_MINUTES`].
async fn pass_detail_rechecks(resources: &ServerContext, user_id: Uuid) {
    exec_for_user(
        resources,
        "UPDATE cached_activities SET detail_recheck_at = $1 \
         WHERE user_id = $2 AND detail_recheck_at IS NOT NULL",
        Utc::now() - Duration::minutes(1),
        user_id,
    )
    .await;
}

/// The recheck instant stored with one cached activity's detail read: `None`
/// when it stands (or none is stored).
async fn detail_recheck_at(
    resources: &ServerContext,
    user_id: Uuid,
    activity_id: &str,
) -> Option<DateTime<Utc>> {
    const SQL: &str = "SELECT detail_recheck_at FROM cached_activities \
                       WHERE user_id = $1 AND activity_id = $2";
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => sqlx::query_scalar(SQL)
            .bind(user_id.to_string())
            .bind(activity_id)
            .fetch_one(sqlite.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => sqlx::query_scalar(SQL)
            .bind(user_id.to_string())
            .bind(activity_id)
            .fetch_one(postgres.pool())
            .await
            .unwrap(),
    }
}

/// Assert the activity's stored read is an `unavailable` answer that expires
/// a recheck period after it was made, somewhere between `before` and now.
async fn assert_stored_unavailable(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    provider: &str,
    activity_id: &str,
    before: DateTime<Utc>,
) {
    let recheck = Duration::minutes(UNREAD_ROUTE_RECHECK_MINUTES);
    let Some(StoredRouteTrack::Unavailable {
        source,
        reason,
        expires_at: Some(expires_at),
    }) = stored(resources, athlete, provider, activity_id).await
    else {
        panic!("{activity_id}: expected an expiring unavailable read");
    };
    assert_eq!(
        (source.as_str(), reason.as_str()),
        ("streams", "unavailable")
    );
    assert!(
        before + recheck <= expires_at && expires_at <= Utc::now() + recheck,
        "{activity_id} expires {expires_at}, a recheck period after the read"
    );
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

/// How many times the mock was asked for a detail and for streams, and the
/// most requests it was answering at one moment.
#[derive(Default)]
struct Hits {
    detail: AtomicUsize,
    streams: AtomicUsize,
    answering: AtomicUsize,
    most_answering: AtomicUsize,
}

impl Hits {
    fn counts(&self) -> (usize, usize) {
        (
            self.detail.load(Ordering::SeqCst),
            self.streams.load(Ordering::SeqCst),
        )
    }

    fn most_at_once(&self) -> usize {
        self.most_answering.load(Ordering::SeqCst)
    }

    /// Hold one request for `latency`, counted as being answered meanwhile.
    async fn answer_after(&self, latency: StdDuration) {
        let answering = self.answering.fetch_add(1, Ordering::SeqCst) + 1;
        self.most_answering.fetch_max(answering, Ordering::SeqCst);
        sleep(latency).await;
        self.answering.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A mock answering `/activities/{id}` and `/activities/{id}/streams` with
/// `streams` as the keyed stream set.
async fn mock_strava(streams: Value) -> (String, Arc<Hits>) {
    mock_strava_answering_after(streams, StdDuration::ZERO).await
}

/// [`mock_strava`], taking `latency` over every answer: long enough that
/// requests made together are at the provider together unless something
/// keeps them apart.
async fn mock_strava_answering_after(streams: Value, latency: StdDuration) -> (String, Arc<Hits>) {
    mock_strava_serving(ride_detail(), streams, latency, 0).await
}

/// [`mock_strava`] whose first `failures` streams requests answer 503, as a
/// streams request Strava could not serve at that moment does.
async fn mock_strava_failing_streams_first(streams: Value, failures: usize) -> (String, Arc<Hits>) {
    mock_strava_serving(ride_detail(), streams, StdDuration::ZERO, failures).await
}

/// [`mock_strava`] whose detail answer is `detail`.
async fn mock_strava_detailing(detail: Value, streams: Value) -> (String, Arc<Hits>) {
    mock_strava_serving(detail, streams, StdDuration::ZERO, 0).await
}

/// Strava's detail answer for the long ride every Strava case caches.
fn ride_detail() -> Value {
    json!({
        "id": 55_001,
        "name": "Long ride",
        "type": "Ride",
        "sport_type": "Ride",
        "start_date": (Utc::now() - Duration::days(2)).to_rfc3339(),
        "elapsed_time": 9_000,
        "distance": 60_000.0
    })
}

async fn mock_strava_serving(
    detail: Value,
    streams: Value,
    latency: StdDuration,
    failures: usize,
) -> (String, Arc<Hits>) {
    let hits = Arc::new(Hits::default());
    let detail_hits = Arc::clone(&hits);
    let stream_hits = Arc::clone(&hits);
    let app = Router::new()
        .route(
            "/activities/{id}",
            get(move || {
                let hits = Arc::clone(&detail_hits);
                let detail = detail.clone();
                async move {
                    hits.detail.fetch_add(1, Ordering::SeqCst);
                    hits.answer_after(latency).await;
                    Json(detail)
                }
            }),
        )
        .route(
            "/activities/{id}/streams",
            get(move || {
                let hits = Arc::clone(&stream_hits);
                let streams = streams.clone();
                async move {
                    let served = hits.streams.fetch_add(1, Ordering::SeqCst);
                    hits.answer_after(latency).await;
                    if served < failures {
                        return Err(HttpStatus::SERVICE_UNAVAILABLE);
                    }
                    Ok(Json(streams))
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

/// A Strava-shaped mock whose streams endpoint answers `404`, as Strava does
/// for a manual entry: the activity exists and holds no samples.
async fn mock_strava_manual_entry() -> (String, Arc<Hits>) {
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
                        "id": 55_021,
                        "name": "Spin class",
                        "type": "Ride",
                        "sport_type": "Ride",
                        "start_date": (Utc::now() - Duration::days(2)).to_rfc3339(),
                        "elapsed_time": 3_600,
                        "distance": 30_000.0,
                        "manual": true
                    }))
                }
            }),
        )
        .route(
            "/activities/{id}/streams",
            get(move || {
                let hits = Arc::clone(&stream_hits);
                async move {
                    hits.streams.fetch_add(1, Ordering::SeqCst);
                    (
                        HttpStatus::NOT_FOUND,
                        Json(json!({
                            "message": "Resource Not Found",
                            "errors": [{ "resource": "Activity", "field": "", "code": "not found" }]
                        })),
                    )
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

/// A mock of a provider API that counts every request it is sent and
/// answers none of them usefully: a provider integration with no stream
/// source must never be asked for a route.
async fn mock_counting_every_request() -> (String, Arc<AtomicUsize>) {
    let requests = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&requests);
    let app = Router::new().fallback(move || {
        let counted = Arc::clone(&counted);
        async move {
            counted.fetch_add(1, Ordering::SeqCst);
            HttpStatus::INTERNAL_SERVER_ERROR
        }
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), requests)
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
    link_oauth(resources, athlete, "strava").await;
}

/// Connect `provider` over OAuth with a live access token.
async fn link_oauth(resources: &Arc<ServerContext>, athlete: &Athlete, provider: &str) {
    let repos = &resources.common.repos;
    repos
        .provider_connections
        .register_connection(
            athlete.user_id,
            athlete.tenant,
            provider,
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
            provider: provider.to_owned(),
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
// A stand-in for the scraper service a mirror backend reads through
// ---------------------------------------------------------------------------

/// A scraper stand-in answering one activity's detail read. The first
/// `misses` answers carry no route, as a detail page scraped before its map
/// rendered does; every later one carries `track`. Returns the base URL and
/// the count of detail reads.
async fn mock_scraper_missing_the_route_first(
    track: &[(f64, f64)],
    misses: usize,
) -> (String, Arc<AtomicUsize>) {
    let reads = Arc::new(AtomicUsize::new(0));
    let detail_reads = Arc::clone(&reads);
    let route = json!({
        "coordinates": track.iter().map(|&(lat, lon)| json!([lat, lon])).collect::<Vec<_>>()
    });
    let app = Router::new()
        .route(
            "/auth/import-session",
            post(|| async { Json(json!({ "session_id": "cap-verified-session" })) }),
        )
        .route(
            "/api/activities/{id}",
            get(move |Path(id): Path<String>| {
                let reads = Arc::clone(&detail_reads);
                let route = route.clone();
                async move {
                    let served = reads.fetch_add(1, Ordering::SeqCst);
                    let mut detail = json!({
                        "id": id,
                        "name": "Sortie du matin",
                        "sport_type": "ride",
                        "start_date": (Utc::now() - Duration::days(2)).to_rfc3339(),
                        "duration_seconds": 9_000,
                        "distance_meters": 60_000.0,
                        "provider": "strava"
                    });
                    if served >= misses {
                        detail["route"] = route;
                    }
                    Json(detail)
                }
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), reads)
}

/// Connect the athlete's Strava through the mirror backend, with a live
/// scrape session.
async fn link_strava_mirror(resources: &Arc<ServerContext>, athlete: &Athlete) {
    resources
        .common
        .repos
        .provider_connections
        .register_connection(
            athlete.user_id,
            athlete.tenant,
            "sciotte",
            &ConnectionType::Manual,
            None,
        )
        .await
        .unwrap();
    seed_sciotte_session(resources, athlete.user_id, athlete.tenant).await;
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
    // The map carries no caption: the Home row and the activity view both
    // name the activity above it, and a caption printed it twice.
    assert!(route["title"].is_null(), "{route}");
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
    assert!(route["title"].is_null(), "{route}");
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

/// [`ride_detail`] carrying two splits and one lap, as Strava's detail answer
/// for a recorded ride does.
fn ride_detail_with_splits_and_laps() -> Value {
    let mut detail = ride_detail();
    detail["splits_metric"] = json!([
        { "split": 1, "distance": 1_000.0, "elapsed_time": 262, "moving_time": 258,
          "elevation_difference": 4.5, "average_speed": 3.75 },
        { "split": 2, "distance": 1_000.0, "elapsed_time": 251, "moving_time": 251,
          "elevation_difference": -1.5, "average_speed": 4.0 }
    ]);
    detail["laps"] = json!([
        { "id": 7_001, "distance": 60_000.0, "elapsed_time": 9_000, "moving_time": 8_820,
          "total_elevation_gain": 410.0, "average_speed": 6.75, "max_speed": 14.25,
          "average_heartrate": 141.0, "max_heartrate": 172.0, "average_watts": 205.0 }
    ]);
    detail
}

/// The activity view's `splits` and `laps` for [`ride_detail_with_splits_and_laps`].
fn assert_view_shows_the_ride_detail(body: &Value) {
    assert_eq!(
        body["splits"],
        json!([
            {
                "index": 1,
                "distance_meters": 1_000.0,
                "elapsed_time_seconds": 262,
                "moving_time_seconds": 258,
                "elevation_difference_meters": 4.5,
                "average_speed_mps": 3.75,
                "average_heart_rate": null
            },
            {
                "index": 2,
                "distance_meters": 1_000.0,
                "elapsed_time_seconds": 251,
                "moving_time_seconds": 251,
                "elevation_difference_meters": -1.5,
                "average_speed_mps": 4.0,
                "average_heart_rate": null
            }
        ]),
        "{body}"
    );
    assert_eq!(
        body["laps"],
        json!([{
            "index": 1,
            "distance_meters": 60_000.0,
            "elapsed_time_seconds": 9_000,
            "moving_time_seconds": 8_820,
            "elevation_gain_meters": 410.0,
            "average_speed_mps": 6.75,
            "average_heart_rate": 141,
            "max_heart_rate": 172,
            "average_power": 205
        }]),
        "{body}"
    );
}

/// The detail read a route took carries the ride's splits and laps. They are
/// kept beside the cached row, so the next list sync — which carries neither
/// and rewrites the row whole — does not take them from the activity's view,
/// and the view, finding the detail read, asks the provider nothing.
#[tokio::test]
#[serial]
async fn the_splits_and_laps_a_route_read_carried_survive_the_next_list_sync() {
    let recorded = weaving_track(900, 0.0);
    let (api_base, hits) =
        mock_strava_detailing(ride_detail_with_splits_and_laps(), streams_for(&recorded)).await;
    let _env = strava_at(api_base);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "route-splits").await;
    link_strava(&resources, &athlete).await;
    let listed = [run_without_overview("55001")];
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "strava",
        &listed,
    )
    .await;

    let (status, route) = route_of(&resources, &athlete.token, "strava", "55001").await;
    assert_eq!(status, StatusCode::OK, "{route}");
    assert_eq!(hits.counts(), (1, 1), "one detail read, one streams read");

    // The next list sync writes the list copy again, without splits or laps.
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "strava",
        &listed,
    )
    .await;

    let (status, body) = get_json(
        &resources,
        &athlete.token,
        "/api/me/activities/strava/55001",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_view_shows_the_ride_detail(&body);
    assert_eq!(
        hits.counts(),
        (1, 1),
        "the view is served from the cache, asking the provider nothing"
    );
}

/// A ride whose route comes from its own overview (Strava's
/// `summary_polyline`) is drawn without any provider call, so no route read
/// ever reads its detail. Its view reads it, once: the splits and laps show
/// on the first open, survive the next list sync, and no later open asks the
/// provider again.
#[tokio::test]
#[serial]
async fn a_ride_drawn_from_its_overview_gets_its_splits_and_laps_from_its_view() {
    let (api_base, hits) = mock_strava_detailing(
        ride_detail_with_splits_and_laps(),
        streams_for(&weaving_track(900, 0.0)),
    )
    .await;
    let _env = strava_at(api_base);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "overview-splits").await;
    link_strava(&resources, &athlete).await;
    let listed = [run_with_overview(
        "55001",
        "strava",
        &weaving_track(300, 0.0),
    )];
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "strava",
        &listed,
    )
    .await;
    let view = "/api/me/activities/strava/55001";

    let (status, route) = route_of(&resources, &athlete.token, "strava", "55001").await;
    assert_eq!(status, StatusCode::OK, "{route}");
    assert!(route["route"].is_object(), "{route}");
    assert_eq!(
        hits.counts(),
        (0, 0),
        "the overview draws it: no provider call"
    );

    let (status, first) = get_json(&resources, &athlete.token, view).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_view_shows_the_ride_detail(&first);
    assert_eq!(hits.counts(), (1, 0), "one detail read, no streams");

    // The next list sync writes the list copy again, without splits or laps.
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "strava",
        &listed,
    )
    .await;

    let (status, again) = get_json(&resources, &athlete.token, view).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_view_shows_the_ride_detail(&again);
    assert_eq!(
        hits.counts(),
        (1, 0),
        "the stored detail answers every later open"
    );

    // A detail that carried splits and laps settles it: no recheck instant,
    // and moving every recheck past leaves it answering.
    assert_eq!(
        detail_recheck_at(&resources, athlete.user_id, "55001").await,
        None
    );
    pass_detail_rechecks(&resources, athlete.user_id).await;
    let (status, later) = get_json(&resources, &athlete.token, view).await;
    assert_eq!(status, StatusCode::OK, "{later}");
    assert_view_shows_the_ride_detail(&later);
    assert_eq!(hits.counts(), (1, 0), "a detail with splits stands");
}

/// A detail read that carried neither splits nor laps answers the view
/// without a provider read on every open, but only until its recheck: a
/// provider serves its activity without them when the request carrying them
/// failed (the Garmin API's `/laps` and `/splits`), so it proves nothing, and
/// past [`EMPTY_DETAIL_RECHECK_MINUTES`] the next open reads it again.
#[tokio::test]
#[serial]
async fn a_detail_read_with_no_splits_or_laps_is_read_again_after_its_recheck() {
    let (api_base, hits) =
        mock_strava_detailing(ride_detail(), streams_for(&weaving_track(900, 0.0))).await;
    let _env = strava_at(api_base);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "overview-no-splits").await;
    link_strava(&resources, &athlete).await;
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "strava",
        &[run_with_overview(
            "55001",
            "strava",
            &weaving_track(300, 0.0),
        )],
    )
    .await;
    let view = "/api/me/activities/strava/55001";

    let before = Utc::now();
    for open in 0..3 {
        let (status, body) = get_json(&resources, &athlete.token, view).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["splits"], json!([]), "open {open}");
        assert_eq!(body["laps"], json!([]), "open {open}");
    }
    assert_eq!(hits.counts(), (1, 0), "one detail read across three opens");
    let recheck = detail_recheck_at(&resources, athlete.user_id, "55001")
        .await
        .expect("an empty detail is stored with its recheck instant");
    let interval = Duration::minutes(EMPTY_DETAIL_RECHECK_MINUTES);
    assert!(
        recheck >= before + interval && recheck <= Utc::now() + interval,
        "{recheck} is {EMPTY_DETAIL_RECHECK_MINUTES} minutes after the read"
    );

    pass_detail_rechecks(&resources, athlete.user_id).await;
    let (status, body) = get_json(&resources, &athlete.token, view).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        hits.counts(),
        (2, 0),
        "past its recheck the detail is read again"
    );
    let (_, body) = get_json(&resources, &athlete.token, view).await;
    assert_eq!(body["laps"], json!([]));
    assert_eq!(
        hits.counts(),
        (2, 0),
        "the read again answers until its own recheck"
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
            expires_at: None,
        }),
        "streams without coordinates are the provider's word: the answer never expires"
    );
    assert!(!listed_has_gps(&resources, &athlete.token, "55002").await);
}

/// A manual Strava entry has no samples: Strava's streams endpoint answers
/// `404`. That is Strava's word that nothing was recorded, so it settles
/// `no_gps` for good, instead of "map could not be loaded" and a detail and a
/// streams call against the app-wide rate limit every ten minutes.
#[tokio::test]
#[serial]
async fn a_manual_strava_entry_whose_streams_are_a_404_answers_no_gps_and_stands() {
    let (api_base, hits) = mock_strava_manual_entry().await;
    let _env = strava_at(api_base);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "route-manual").await;
    link_strava(&resources, &athlete).await;
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "strava",
        &[ride_without_position("55021")],
    )
    .await;

    let (status, body) = route_of(&resources, &athlete.token, "strava", "55021").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "route": null, "reason": "no_gps" }));
    assert_eq!(hits.counts(), (1, 1));
    assert_eq!(
        stored(&resources, &athlete, "strava", "55021").await,
        Some(StoredRouteTrack::Unavailable {
            source: "streams".to_owned(),
            reason: "no_gps".to_owned(),
            expires_at: None,
        }),
        "a streams 404 settles the answer: it never expires"
    );
    assert!(!listed_has_gps(&resources, &athlete.token, "55021").await);
    let (_, again) = route_of(&resources, &athlete.token, "strava", "55021").await;
    assert_eq!(again, body);
    assert_eq!(hits.counts(), (1, 1), "never read again");
}

/// WHOOP serves a workout without samples, whatever it recorded: the
/// integration has no stream source. Its route is settled `no_gps` without a
/// single request to WHOOP, and stands — never "map could not be loaded"
/// with a WHOOP read every ten minutes.
#[cfg(feature = "provider-whoop")]
#[tokio::test]
#[serial]
async fn a_provider_with_no_stream_source_answers_no_gps_without_a_read() {
    let (api_base, requests) = mock_counting_every_request().await;
    let _env = EnvGuard::set(&[
        ("PIERRE_WHOOP_API_BASE_URL", api_base.clone()),
        ("PIERRE_WHOOP_TOKEN_URL", format!("{api_base}/oauth/token")),
        ("WHOOP_CLIENT_ID", "whoop_client".to_owned()),
        ("WHOOP_CLIENT_SECRET", "whoop_secret".to_owned()),
    ]);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "route-whoop").await;
    link_oauth(&resources, &athlete, "whoop").await;
    let workout = ActivityBuilder::new(
        "whoop-7",
        "Functional fitness",
        SportType::Workout,
        Utc::now() - Duration::days(1),
        2_700,
        "whoop",
    )
    .build();
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "whoop",
        &[workout],
    )
    .await;

    let (status, body) = route_of(&resources, &athlete.token, "whoop", "whoop-7").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "route": null, "reason": "no_gps" }));
    assert_eq!(
        requests.load(Ordering::SeqCst),
        0,
        "WHOOP was asked nothing"
    );
    assert_eq!(
        stored(&resources, &athlete, "whoop", "whoop-7").await,
        Some(StoredRouteTrack::Unavailable {
            source: "streams".to_owned(),
            reason: "no_gps".to_owned(),
            expires_at: None,
        })
    );
    assert!(!listed_has_gps(&resources, &athlete.token, "whoop-7").await);
    assert_eq!(requests.load(Ordering::SeqCst), 0);
}

// ---------------------------------------------------------------------------
// A detail read with no stream set proves nothing: it is unavailable, briefly
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn a_mirror_detail_read_with_no_route_is_unavailable_and_read_again_once_it_expires() {
    let recorded = weaving_track(900, 0.0);
    let (scraper, reads) = mock_scraper_missing_the_route_first(&recorded, 1).await;
    let _env = EnvGuard::set(&[("DRAVR_SCIOTTE_REMOTE_URL", scraper)]);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "route-mirror-miss").await;
    link_strava_mirror(&resources, &athlete).await;
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "sciotte",
        &[ride_without_position("m2")],
    )
    .await;

    let before = Utc::now();
    let (status, body) = route_of(&resources, &athlete.token, "strava", "m2").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "route": null, "reason": "unavailable" }));
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    assert_stored_unavailable(&resources, &athlete, "sciotte", "m2", before).await;
    assert!(
        listed_has_gps(&resources, &athlete.token, "m2").await,
        "a read that settled nothing never tells the list the ride has no GPS"
    );
    let (_, again) = route_of(&resources, &athlete.token, "strava", "m2").await;
    assert_eq!(again, body);
    assert_eq!(
        reads.load(Ordering::SeqCst),
        1,
        "until it expires, the stored answer is not read again"
    );

    expire_route_reads(&resources, athlete.user_id).await;
    assert!(
        listed_has_gps(&resources, &athlete.token, "m2").await,
        "an expired answer sends the client back to the route"
    );
    let (status, body) = route_of(&resources, &athlete.token, "strava", "m2").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(reads.load(Ordering::SeqCst), 2, "the route is read again");
    assert!(body["reason"].is_null(), "{body}");
    let drawn = coordinates(&body["route"]);
    assert!(drawn.len() > 2 && drawn.len() <= HOME_ROUTE_MAX_POINTS);
    assert!(metres(drawn[0], recorded[0]) >= DEFAULT_PRIVACY_RADIUS_METERS);
    assert!(matches!(
        stored(&resources, &athlete, "sciotte", "m2").await,
        Some(StoredRouteTrack::Drawn { ref source, .. }) if source == "streams"
    ));
    assert!(listed_has_gps(&resources, &athlete.token, "m2").await);
    let (_, stored_answer) = route_of(&resources, &athlete.token, "strava", "m2").await;
    assert_eq!(stored_answer, body);
    assert_eq!(reads.load(Ordering::SeqCst), 2, "a drawn track stands");
}

#[tokio::test]
#[serial]
async fn a_streams_request_that_failed_answers_unavailable_until_it_expires() {
    let recorded = weaving_track(900, 0.0);
    let (api_base, hits) = mock_strava_failing_streams_first(streams_for(&recorded), 1).await;
    let _env = strava_at(api_base);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "route-streams-failed").await;
    link_strava(&resources, &athlete).await;
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "strava",
        &[ride_without_position("55011")],
    )
    .await;

    let before = Utc::now();
    let (status, body) = route_of(&resources, &athlete.token, "strava", "55011").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "route": null, "reason": "unavailable" }));
    assert_eq!(hits.counts(), (1, 1));
    assert_stored_unavailable(&resources, &athlete, "strava", "55011", before).await;
    assert!(listed_has_gps(&resources, &athlete.token, "55011").await);

    expire_route_reads(&resources, athlete.user_id).await;
    let (status, body) = route_of(&resources, &athlete.token, "strava", "55011").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(hits.counts(), (2, 2), "the expired answer is read again");
    assert!(coordinates(&body["route"]).len() > 2, "{body}");
    assert!(matches!(
        stored(&resources, &athlete, "strava", "55011").await,
        Some(StoredRouteTrack::Drawn { ref source, .. }) if source == "streams"
    ));
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
// A list row with no position: the streams settle it, and the list follows
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn a_row_with_no_start_and_no_overview_is_drawn_from_its_streams() {
    let recorded = weaving_track(900, 0.0);
    let (api_base, hits) = mock_strava(streams_for(&recorded)).await;
    let _env = strava_at(api_base);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "route-bare").await;
    link_strava(&resources, &athlete).await;
    let ride = ride_without_position("55004");
    assert!(ride.start_latitude().is_none() && ride.start_longitude().is_none());
    assert!(ride.summary_polyline().is_none());
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "strava",
        &[ride],
    )
    .await;

    assert!(
        listed_has_gps(&resources, &athlete.token, "55004").await,
        "the list sends the client to the route endpoint"
    );
    assert_eq!(hits.counts(), (0, 0), "the list reads no provider");

    let (status, body) = route_of(&resources, &athlete.token, "strava", "55004").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(hits.counts(), (1, 1), "one detail read, one streams read");
    assert!(body["reason"].is_null());
    let route = &body["route"];
    let drawn = coordinates(route);
    assert!(drawn.len() > 2 && drawn.len() <= HOME_ROUTE_MAX_POINTS);
    assert!(metres(drawn[0], recorded[0]) >= DEFAULT_PRIVACY_RADIUS_METERS);
    assert!(
        metres(*drawn.last().unwrap(), *recorded.last().unwrap()) >= DEFAULT_PRIVACY_RADIUS_METERS
    );
    assert_eq!(
        route["elevation_meters"].as_array().unwrap().len(),
        drawn.len()
    );
    assert!(matches!(
        stored(&resources, &athlete, "strava", "55004").await,
        Some(StoredRouteTrack::Drawn { ref source, .. }) if source == "streams"
    ));
    assert!(listed_has_gps(&resources, &athlete.token, "55004").await);
}

#[tokio::test]
#[serial]
async fn a_row_with_no_position_whose_streams_hold_no_gps_leaves_the_list_saying_so() {
    let (api_base, hits) = mock_strava(streams_for(&[])).await;
    let _env = strava_at(api_base);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "route-bare-indoor").await;
    link_strava(&resources, &athlete).await;
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "strava",
        &[ride_without_position("55005")],
    )
    .await;

    assert!(listed_has_gps(&resources, &athlete.token, "55005").await);
    let (status, body) = route_of(&resources, &athlete.token, "strava", "55005").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "route": null, "reason": "no_gps" }));
    assert!(
        !listed_has_gps(&resources, &athlete.token, "55005").await,
        "the stored read is what the list reads"
    );
    assert_eq!(hits.counts(), (1, 1), "the list reads no provider");
}

// ---------------------------------------------------------------------------
// One athlete's reads take turns at the provider
// ---------------------------------------------------------------------------

/// Long enough for requests made together to overlap at the mock.
const PROVIDER_LATENCY: StdDuration = StdDuration::from_millis(120);

#[tokio::test]
#[serial]
async fn two_requests_at_once_for_one_activity_cost_one_provider_read() {
    let recorded = weaving_track(900, 0.0);
    let (api_base, hits) =
        mock_strava_answering_after(streams_for(&recorded), PROVIDER_LATENCY).await;
    let _env = strava_at(api_base);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "route-twice").await;
    link_strava(&resources, &athlete).await;
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "strava",
        &[ride_without_position("55006")],
    )
    .await;

    let ((first_status, first), (second_status, second)) = tokio::join!(
        route_of(&resources, &athlete.token, "strava", "55006"),
        route_of(&resources, &athlete.token, "strava", "55006"),
    );
    assert_eq!(first_status, StatusCode::OK, "{first}");
    assert_eq!(second_status, StatusCode::OK, "{second}");
    assert!(coordinates(&first["route"]).len() > 2);
    assert_eq!(
        second, first,
        "the request that waited answers what the read stored"
    );
    assert_eq!(
        hits.counts(),
        (1, 1),
        "one detail read and one streams read between the two requests"
    );
}

#[tokio::test]
#[serial]
async fn one_athletes_routes_are_read_from_the_provider_one_at_a_time() {
    let recorded = weaving_track(900, 0.0);
    let (api_base, hits) =
        mock_strava_answering_after(streams_for(&recorded), PROVIDER_LATENCY).await;
    let _env = strava_at(api_base);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "route-turns").await;
    link_strava(&resources, &athlete).await;
    let ids = ["55007", "55008", "55009"];
    let rides: Vec<Activity> = ids.iter().map(|id| ride_without_position(id)).collect();
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "strava",
        &rides,
    )
    .await;

    let answers = join_all(
        ids.iter()
            .map(|id| route_of(&resources, &athlete.token, "strava", id)),
    )
    .await;
    for (status, body) in &answers {
        assert_eq!(*status, StatusCode::OK, "{body}");
        assert!(coordinates(&body["route"]).len() > 2);
    }
    assert_eq!(hits.counts(), (3, 3), "each activity is read once");
    assert_eq!(
        hits.most_at_once(),
        1,
        "the provider answers one of the athlete's reads at a time"
    );
    for id in ids {
        assert!(matches!(
            stored(&resources, &athlete, "strava", id).await,
            Some(StoredRouteTrack::Drawn { ref source, .. }) if source == "streams"
        ));
    }
}

#[tokio::test]
#[serial]
async fn two_athletes_do_not_wait_for_each_other() {
    let recorded = weaving_track(900, 0.0);
    let (api_base, hits) =
        mock_strava_answering_after(streams_for(&recorded), PROVIDER_LATENCY).await;
    let _env = strava_at(api_base);
    let resources = common::create_test_server_resources().await.unwrap();
    let one = seed_athlete(&resources, "route-one").await;
    let other = seed_athlete(&resources, "route-other").await;
    for athlete in [&one, &other] {
        link_strava(&resources, athlete).await;
        cache(
            &resources,
            athlete.user_id,
            athlete.tenant,
            "strava",
            &[ride_without_position("55010")],
        )
        .await;
    }

    let ((one_status, one_body), (other_status, other_body)) = tokio::join!(
        route_of(&resources, &one.token, "strava", "55010"),
        route_of(&resources, &other.token, "strava", "55010"),
    );
    assert_eq!(one_status, StatusCode::OK, "{one_body}");
    assert_eq!(other_status, StatusCode::OK, "{other_body}");
    assert_eq!(hits.counts(), (2, 2), "each athlete's own read");
    assert_eq!(
        hits.most_at_once(),
        2,
        "a turn is one athlete's: another athlete reads meanwhile"
    );
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
