// ABOUTME: End to end, Home against a scripted scraper speaking sciotte's contract: only a list the scraper vouched for is a sync
// ABOUTME: Pins the 2026-09-29 incident, shed/refused/lost sessions, a restarted scraper, route reads' bounds and turns, and no_gps

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Home sync-truth suite (carnet#656).
//!
//! Drives the real Home router — `GET /api/me/activities/recent` and the
//! activity route read — and the chat's `get_activities` tool for an athlete
//! whose Strava is read through the sciotte mirror, pointed
//! (`DRAVR_SCIOTTE_REMOTE_URL`) at a scripted stand-in for the scraper
//! service. The stand-in speaks the scraper's REST contract
//! (`/auth/import-session`, `/api/activities`, `/api/activities/{id}`): it
//! holds sessions in memory as the scraper does (a read naming one it does
//! not hold is `401 session_not_found`, a restart forgets them all), builds
//! every body with the scraper's own types and serializer (`dravr_sciotte`),
//! and answers errors in the scraper service's shapes (`503 scraper_busy`
//! with `Retry-After`, `401 session_expired`, `500 {"error": <message>}`).
//! It can play a list it vouches for, an empty list, a failed list, a capture
//! missing its head, a list that never answers, the 2026-09-29 list walk
//! killed by a detail read on the same session, a detail with a route,
//! without one, with one that holds no coordinates, a slow one or one that
//! never answers — and it can go down and come back. Every assertion is on
//! what Home or the chat answers and what the scraper was asked, so a
//! platform that took a failed scrape for a sync, or a failed route read for
//! "no GPS", fails on content.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
#![cfg(all(feature = "provider-sciotte", feature = "protocol-rest"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
#[path = "helpers/sciotte_mock.rs"]
mod sciotte_mock;

use std::collections::HashSet;
use std::env;
use std::future::pending;
use std::iter::once;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration as StdDuration, Instant};

use axum::body::{to_bytes, Body};
use axum::extract::{Path, RawQuery, State};
use axum::http::{header, HeaderMap, HeaderValue, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Duration, TimeZone, Utc};
use dravr_sciotte::error::ScraperError;
use dravr_sciotte::models::{Activity as ScrapedActivity, RouteTrack as ScrapedRoute};
use dravr_sciotte::wire::{ActivitiesResponse, SCRAPER_BUSY, SESSION_EXPIRED, SESSION_NOT_FOUND};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use futures_util::future::join_all;
use pierre_core::models::{
    Activity, ActivityBuilder, ConnectionStatus, ConnectionType, SportType, TenantId,
};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::StoredRouteTrack;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use pierre_mcp_server::services::activity_route::UNREAD_ROUTE_RECHECK_MINUTES;
use pierre_providers::core::ActivityQueryParams;
use pierre_tool_runtime::activity_fetch::{fetch_provider_activities, fetch_provider_head};
use pierre_tool_runtime::capture_sweep::{refresh_captures, RefreshOutcome, SweepBudget};
use pierre_tool_runtime::implementations::data::GetActivitiesTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use serial_test::serial;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::sleep;
use tower::ServiceExt;
use uuid::Uuid;

use crate::sciotte_mock::seed_sciotte_session;

/// Parc La Fontaine, Montréal — where the scripted route starts.
const HOME: (f64, f64) = (45.5259, -73.5697);

// ---------------------------------------------------------------------------
// The scripted scraper
// ---------------------------------------------------------------------------

/// What a list scrape whose page died under it answered.
#[derive(Clone, Copy)]
enum DeadPage {
    /// `200` with `count: 0` and `head_complete: true`: what dravr-sciotte
    /// v0.19.0 answered on 2026-09-29, when a detail read closed the browser
    /// the list was walking.
    AnsweredEmpty,
    /// The scraper's error answer for a list walk that read nothing, as a
    /// scraper that treats a failed walk as an error sends it.
    AnsweredError,
}

/// What the stand-in answers `GET /api/activities` with.
#[derive(Clone)]
enum ListScript {
    /// `count` and `activities` agree, `head_complete: true`.
    Rows(Vec<ScrapedActivity>),
    /// The rows, with `head_complete: false`: the fresh-head fetch failed.
    Incomplete(Vec<ScrapedActivity>),
    /// A `500`, as a scrape whose browser was closed under it.
    Fails,
    /// `count` announces more rows than the body carries.
    Truncated(Vec<ScrapedActivity>),
    /// No answer at all: a scrape stuck on a browser that never launches.
    Hangs,
    /// The scraper shed the read: `503 scraper_busy` with its wait.
    Busy,
    /// The provider refused the session's cookies: `401 session_expired`.
    SessionExpired,
    /// The incident: the list walk waits until a detail read on the same
    /// session finishes — that read closes the browser the walk shares — and
    /// the dead page then answers as `DeadPage` says.
    DiesUnderADetail(DeadPage),
}

/// What the stand-in answers `GET /api/activities/{id}` with.
#[derive(Clone, Copy)]
enum DetailScript {
    /// A detail carrying a GPS route.
    Route,
    /// A detail with no `route` at all: the scraper read no route.
    NoRoute,
    /// A detail whose `route` holds no coordinates: the scraper read the
    /// page and the activity recorded no GPS.
    EmptyRoute,
    /// A detail carrying a GPS route, answered after this many milliseconds.
    SlowRoute(u64),
    /// A `500`.
    Fails,
    /// No answer at all.
    Hangs,
}

/// One running instance of the stand-in: how to stop it.
struct Instance {
    shutdown: oneshot::Sender<()>,
    serving: JoinHandle<()>,
}

/// The stand-in's script and what it was asked.
struct Scraper {
    list: Mutex<ListScript>,
    detail: Mutex<DetailScript>,
    list_reads: AtomicUsize,
    detail_reads: AtomicUsize,
    /// The query string of every list read, in order.
    list_queries: Mutex<Vec<String>>,
    /// The sessions this instance holds. An import adds one; a restart
    /// forgets them all, as the scraper's in-memory store does.
    held: Mutex<HashSet<String>>,
    imports: AtomicUsize,
    /// Answer the next import and forget its session at once: the instance
    /// that took the import went down, and the read reaches its replacement.
    forget_next_import: AtomicBool,
    /// Reads on the session in flight now, and the most there ever were.
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
    /// Detail reads in flight now, and the most there ever were.
    details_in_flight: AtomicUsize,
    max_details_in_flight: AtomicUsize,
    /// Detail reads that have answered.
    details_answered: AtomicUsize,
    /// Where the stand-in listens, kept across a restart.
    addr: SocketAddr,
    running: Mutex<Option<Instance>>,
}

/// A read in flight, counted while it lives.
struct InFlight<'a> {
    count: &'a AtomicUsize,
}

impl<'a> InFlight<'a> {
    fn enter(count: &'a AtomicUsize, most: &AtomicUsize) -> Self {
        let now = count.fetch_add(1, Ordering::SeqCst) + 1;
        most.fetch_max(now, Ordering::SeqCst);
        Self { count }
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Scraper {
    fn play_list(&self, script: ListScript) {
        *self.list.lock().unwrap() = script;
    }

    fn play_detail(&self, script: DetailScript) {
        *self.detail.lock().unwrap() = script;
    }

    fn list_reads(&self) -> usize {
        self.list_reads.load(Ordering::SeqCst)
    }

    fn detail_reads(&self) -> usize {
        self.detail_reads.load(Ordering::SeqCst)
    }

    fn imports(&self) -> usize {
        self.imports.load(Ordering::SeqCst)
    }

    fn max_in_flight(&self) -> usize {
        self.max_in_flight.load(Ordering::SeqCst)
    }

    fn max_details_in_flight(&self) -> usize {
        self.max_details_in_flight.load(Ordering::SeqCst)
    }

    fn list_queries(&self) -> Vec<String> {
        self.list_queries.lock().unwrap().clone()
    }

    /// Whether the read names a session this instance holds.
    fn holds(&self, headers: &HeaderMap) -> bool {
        headers
            .get("x-session-id")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|id| self.held.lock().unwrap().contains(id))
    }

    /// Wait until the scraper has been asked for its list `reads` times.
    async fn await_list_reads(&self, reads: usize) {
        let deadline = Instant::now() + StdDuration::from_secs(10);
        while self.list_reads() < reads {
            assert!(Instant::now() < deadline, "the list was never read");
            sleep(StdDuration::from_millis(10)).await;
        }
    }

    /// Stop the instance: connections close and new ones are refused, as a
    /// scraper that went down refuses them.
    async fn stop(&self) {
        let instance = self.running.lock().unwrap().take();
        if let Some(Instance { shutdown, serving }) = instance {
            shutdown.send(()).ok();
            serving.await.unwrap();
        }
    }

    /// Start a fresh instance on the same address, holding no session: a
    /// restarted scraper has lost every session it held.
    async fn restart(self: &Arc<Self>) {
        self.held.lock().unwrap().clear();
        let listener = TcpListener::bind(self.addr).await.unwrap();
        self.serve(listener);
    }

    fn serve(self: &Arc<Self>, listener: TcpListener) {
        let app = Router::new()
            .route("/auth/import-session", post(import_session))
            .route("/api/activities", get(list_activities))
            .route("/api/activities/{id}", get(activity_detail))
            .with_state(Arc::clone(self));
        let (shutdown, stopped) = oneshot::channel::<()>();
        let serving = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    stopped.await.ok();
                })
                .await
                .unwrap();
        });
        *self.running.lock().unwrap() = Some(Instance { shutdown, serving });
    }
}

/// One scraped ride, as the scraper's own type holds it.
fn scraped_ride(id: &str, started: DateTime<Utc>) -> ScrapedActivity {
    serde_json::from_value(json!({
        "id": id,
        "name": format!("Sortie {id}"),
        "sport_type": "ride",
        "start_date": started.to_rfc3339(),
        "duration_seconds": 5_400,
        "provider": "strava",
        "distance_meters": 42_000.0,
        "elevation_gain": 310.0
    }))
    .unwrap()
}

/// A route of `coordinates`, as the scraper's own type holds it.
const fn scraped_route(coordinates: Vec<(f64, f64)>) -> ScrapedRoute {
    ScrapedRoute {
        coordinates,
        altitudes_meters: None,
        distances_meters: None,
        bounds: None,
    }
}

/// A 900-point track leaving `HOME` north-east with a sideways weave, so
/// the privacy trim leaves a long middle to draw.
fn weaving_track() -> Vec<(f64, f64)> {
    (0..900)
        .map(|i| {
            let step = f64::from(i);
            (
                step.mul_add(0.000_12, HOME.0),
                (step / 40.0)
                    .sin()
                    .mul_add(0.000_5, step.mul_add(0.000_12, HOME.1)),
            )
        })
        .collect()
}

/// A list body, serialized the way the scraper serializes one.
fn list_body(rows: Vec<ScrapedActivity>, count: usize, head_complete: bool) -> Response {
    Json(
        serde_json::to_value(ActivitiesResponse {
            count,
            activities: rows,
            head_complete,
        })
        .unwrap(),
    )
    .into_response()
}

/// The answer the scraper service gives a scraper error, as
/// `dravr-sciotte-server`'s `error_response::scraper_error_response` builds
/// it: `503 scraper_busy` with `Retry-After`, `401 session_expired` for a
/// session the provider refused, and a `500 {"error": <message>}` otherwise.
fn scraper_error_response(error: &ScraperError) -> Response {
    match error {
        ScraperError::Busy {
            reason,
            retry_after_secs,
        } => {
            let mut response = (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "error": SCRAPER_BUSY,
                    "reason": reason,
                    "retry_after_secs": retry_after_secs,
                })),
            )
                .into_response();
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(*retry_after_secs));
            response
        }
        ScraperError::Auth { .. } | ScraperError::SessionExpired { .. } => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": SESSION_EXPIRED, "message": error.to_string() })),
        )
            .into_response(),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

/// The scraper service's `401` for a read naming a session it does not hold.
fn session_not_found() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "error": SESSION_NOT_FOUND,
            "message": "Name a session this service holds in the X-Session-Id header (import it or log in first).",
        })),
    )
        .into_response()
}

fn browser_closed() -> ScraperError {
    ScraperError::Browser {
        reason: "browser closed during the scrape".to_owned(),
    }
}

async fn import_session(
    State(scraper): State<Arc<Scraper>>,
    Json(request): Json<Value>,
) -> Response {
    scraper.imports.fetch_add(1, Ordering::SeqCst);
    let session_id = request["session"]["session_id"]
        .as_str()
        .expect("an import carries the session")
        .to_owned();
    let provider = request["provider"].clone();
    let mut held = scraper.held.lock().unwrap();
    if scraper.forget_next_import.swap(false, Ordering::SeqCst) {
        // The replacement instance the read reaches holds nothing.
        held.remove(&session_id);
    } else {
        held.insert(session_id.clone());
    }
    drop(held);
    Json(json!({ "status": "imported", "session_id": session_id, "provider": provider }))
        .into_response()
}

async fn list_activities(
    State(scraper): State<Arc<Scraper>>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    scraper.list_reads.fetch_add(1, Ordering::SeqCst);
    scraper
        .list_queries
        .lock()
        .unwrap()
        .push(query.unwrap_or_default());
    if !scraper.holds(&headers) {
        return session_not_found();
    }
    let _in_flight = InFlight::enter(&scraper.in_flight, &scraper.max_in_flight);
    let script = scraper.list.lock().unwrap().clone();
    match script {
        ListScript::Rows(rows) => {
            let count = rows.len();
            list_body(rows, count, true)
        }
        ListScript::Incomplete(rows) => {
            let count = rows.len();
            list_body(rows, count, false)
        }
        ListScript::Truncated(rows) => {
            let count = rows.len() + 3;
            list_body(rows, count, true)
        }
        ListScript::Fails => scraper_error_response(&browser_closed()),
        ListScript::Hangs => pending().await,
        ListScript::Busy => scraper_error_response(&ScraperError::Busy {
            reason: "scrape queue full".to_owned(),
            retry_after_secs: 1,
        }),
        ListScript::SessionExpired => scraper_error_response(&ScraperError::SessionExpired {
            reason: "Strava redirected the session to its login page".to_owned(),
        }),
        ListScript::DiesUnderADetail(dead) => {
            let answered = scraper.details_answered.load(Ordering::SeqCst);
            let deadline = Instant::now() + StdDuration::from_secs(10);
            while scraper.details_answered.load(Ordering::SeqCst) == answered {
                if Instant::now() >= deadline {
                    // No detail read overlapped the walk: the scenario never
                    // happened, and the answer says so loudly.
                    return scraper_error_response(&ScraperError::Internal {
                        reason: "no detail read overlapped the list walk".to_owned(),
                    });
                }
                sleep(StdDuration::from_millis(10)).await;
            }
            match dead {
                DeadPage::AnsweredEmpty => list_body(Vec::new(), 0, true),
                DeadPage::AnsweredError => scraper_error_response(&browser_closed()),
            }
        }
    }
}

async fn activity_detail(
    State(scraper): State<Arc<Scraper>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    scraper.detail_reads.fetch_add(1, Ordering::SeqCst);
    if !scraper.holds(&headers) {
        return session_not_found();
    }
    let answer = {
        let _in_flight = InFlight::enter(&scraper.in_flight, &scraper.max_in_flight);
        let _detail = InFlight::enter(&scraper.details_in_flight, &scraper.max_details_in_flight);
        detail_answer(&scraper, &id).await
    };
    scraper.details_answered.fetch_add(1, Ordering::SeqCst);
    answer
}

async fn detail_answer(scraper: &Scraper, id: &str) -> Response {
    let script = *scraper.detail.lock().unwrap();
    let mut detail = scraped_ride(id, Utc::now() - Duration::days(1));
    match script {
        DetailScript::Route => detail.route = Some(scraped_route(weaving_track())),
        DetailScript::SlowRoute(millis) => {
            sleep(StdDuration::from_millis(millis)).await;
            detail.route = Some(scraped_route(weaving_track()));
        }
        DetailScript::EmptyRoute => detail.route = Some(scraped_route(Vec::new())),
        DetailScript::NoRoute => {}
        DetailScript::Fails => {
            return scraper_error_response(&ScraperError::Browser {
                reason: "detail page timed out".to_owned(),
            });
        }
        DetailScript::Hangs => pending().await,
    }
    Json(serde_json::to_value(&detail).unwrap()).into_response()
}

/// Start the scripted scraper. Returns its base URL and its script handle.
async fn spawn_scraper(list: ListScript) -> (String, Arc<Scraper>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let scraper = Arc::new(Scraper {
        list: Mutex::new(list),
        detail: Mutex::new(DetailScript::Route),
        list_reads: AtomicUsize::new(0),
        detail_reads: AtomicUsize::new(0),
        list_queries: Mutex::new(Vec::new()),
        held: Mutex::new(HashSet::new()),
        imports: AtomicUsize::new(0),
        forget_next_import: AtomicBool::new(false),
        in_flight: AtomicUsize::new(0),
        max_in_flight: AtomicUsize::new(0),
        details_in_flight: AtomicUsize::new(0),
        max_details_in_flight: AtomicUsize::new(0),
        details_answered: AtomicUsize::new(0),
        addr,
        running: Mutex::new(None),
    });
    scraper.serve(listener);
    (format!("http://{addr}"), scraper)
}

/// Environment for one test, removed after it: the scraper's URL, and any
/// bound a test shortens so a read that never answers is given up in
/// seconds.
struct TestEnv {
    keys: Vec<&'static str>,
}

impl TestEnv {
    fn point_at(url: String) -> Self {
        Self::with(&[("DRAVR_SCIOTTE_REMOTE_URL", url)])
    }

    fn with(vars: &[(&'static str, String)]) -> Self {
        for (key, value) in vars {
            env::set_var(key, value);
        }
        Self {
            keys: vars.iter().map(|(key, _)| *key).collect(),
        }
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        for key in &self.keys {
            env::remove_var(key);
        }
    }
}

// ---------------------------------------------------------------------------
// The athlete, the Home router and the chat tool
// ---------------------------------------------------------------------------

struct Athlete {
    user_id: Uuid,
    tenant: TenantId,
    token: String,
}

/// A fresh athlete whose Strava is connected through the sciotte mirror with
/// a live scrape session.
async fn athlete_on_the_mirror(resources: &Arc<ServerContext>, label: &str) -> Athlete {
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
        .register_connection(user_id, tenant, "sciotte", &ConnectionType::Manual, None)
        .await
        .unwrap();
    seed_sciotte_session(resources, user_id, tenant).await;
    Athlete {
        user_id,
        tenant,
        token,
    }
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

/// One Home load of the recent list.
async fn home(resources: &Arc<ServerContext>, athlete: &Athlete) -> Value {
    home_at(resources, athlete, "/api/me/activities/recent").await
}

/// One Home load the athlete's retry makes.
async fn home_retry(resources: &Arc<ServerContext>, athlete: &Athlete) -> Value {
    home_at(resources, athlete, "/api/me/activities/recent?retry=true").await
}

async fn home_at(resources: &Arc<ServerContext>, athlete: &Athlete, uri: &str) -> Value {
    let (status, body) = get_json(resources, &athlete.token, uri).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// One route read, through the provider the athlete knows the ride from.
async fn route(resources: &Arc<ServerContext>, athlete: &Athlete, id: &str) -> Value {
    route_at(
        resources,
        athlete,
        &format!("/api/me/activities/strava/{id}/route"),
    )
    .await
}

/// One route read the athlete's retry makes.
async fn route_retry(resources: &Arc<ServerContext>, athlete: &Athlete, id: &str) -> Value {
    route_at(
        resources,
        athlete,
        &format!("/api/me/activities/strava/{id}/route?retry=true"),
    )
    .await
}

async fn route_at(resources: &Arc<ServerContext>, athlete: &Athlete, uri: &str) -> Value {
    let (status, body) = get_json(resources, &athlete.token, uri).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// One `get_activities` call the athlete's coach makes for a window.
async fn chat_activities(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    after: DateTime<Utc>,
) -> Value {
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let ctx = ToolContext::new()
        .with_user(athlete.user_id.to_string())
        .with_tenant(athlete.tenant.to_string())
        .with_auth_method("jwt_bearer");
    let response = GetActivitiesTool
        .execute(
            &runtime,
            &ctx,
            json!({
                "limit": 10,
                "mode": "summary",
                "after": after.timestamp(),
                "before": Utc::now().timestamp()
            }),
        )
        .await;
    response
        .structured_content
        .expect("get_activities answers with structured content")
}

fn ids(body: &Value) -> Vec<String> {
    body["activities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap().to_owned())
        .collect()
}

fn has_gps(body: &Value, id: &str) -> bool {
    body["activities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == id)
        .unwrap_or_else(|| panic!("{id} is not on Home: {body}"))["has_gps"]
        .as_bool()
        .unwrap()
}

fn instant(value: &Value) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(
        value
            .as_str()
            .unwrap_or_else(|| panic!("not an instant: {value}")),
    )
    .unwrap()
    .with_timezone(&Utc)
}

/// When Home says the sync failed, or a panic naming the body.
fn failed_at(body: &Value) -> DateTime<Utc> {
    instant(&body["sync_failure"]["failed_at"])
}

/// Whether two instants are the same to within the storage round trip.
fn same_instant(a: DateTime<Utc>, b: DateTime<Utc>) -> bool {
    (a - b).num_seconds().abs() < 2
}

/// Wait for every task the route spawned on the drain tracker to finish.
async fn await_background(resources: &ServerContext) {
    let deadline = Instant::now() + StdDuration::from_secs(20);
    while !resources.common.turns.is_empty() {
        assert!(
            Instant::now() < deadline,
            "the background refresh did not finish within 20s"
        );
        sleep(StdDuration::from_millis(25)).await;
    }
}

/// Run one SQL statement binding `at` and the athlete's user id, on
/// whichever engine the suite runs on.
async fn exec_at(resources: &ServerContext, sql: &str, at: DateTime<Utc>, athlete: &Athlete) {
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query(sql)
                .bind(at)
                .bind(athlete.user_id.to_string())
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => {
            sqlx::query(sql)
                .bind(at)
                .bind(athlete.user_id.to_string())
                .execute(postgres.pool())
                .await
                .unwrap();
        }
    }
}

/// Age the athlete's last good sync to `at`: every cached row's `synced_at`
/// and the provider's fetch mark, as if both were written then. No repository
/// writes an old `synced_at`, so the fixture writes it in SQL.
async fn age_last_sync(resources: &ServerContext, athlete: &Athlete, at: DateTime<Utc>) {
    exec_at(
        resources,
        "UPDATE cached_activities SET synced_at = $1 WHERE user_id = $2",
        at,
        athlete,
    )
    .await;
    resources
        .common
        .repos
        .activity_cache
        .record_activity_fetch(athlete.user_id, &athlete.tenant, "sciotte", at)
        .await
        .unwrap();
}

/// Move the athlete's recorded sync failure back to `at`, as if its pause had
/// been running since then. A streak that began after `at` now begins there,
/// so the first failure is never later than the last.
async fn age_failure(resources: &ServerContext, athlete: &Athlete, at: DateTime<Utc>) {
    exec_at(
        resources,
        "UPDATE activity_fetch_failures SET failed_at = $1, \
         streak_started_at = CASE WHEN streak_started_at > $1 THEN $1 \
         ELSE streak_started_at END \
         WHERE CAST(user_id AS TEXT) = $2",
        at,
        athlete,
    )
    .await;
}

/// Move every stored route read of the athlete's that carries an expiry to a
/// minute ago, as if its recheck period had passed.
async fn expire_route_reads(resources: &ServerContext, athlete: &Athlete) {
    exec_at(
        resources,
        "UPDATE activity_route_tracks SET expires_at = $1 \
         WHERE user_id = $2 AND expires_at IS NOT NULL",
        Utc::now() - Duration::minutes(1),
        athlete,
    )
    .await;
}

async fn stored_route(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    id: &str,
) -> Option<StoredRouteTrack> {
    resources
        .common
        .repos
        .activity_route_tracks
        .get_route_track(&athlete.tenant, athlete.user_id, "sciotte", id)
        .await
        .unwrap()
}

async fn connection_status(resources: &ServerContext, athlete: &Athlete) -> ConnectionStatus {
    resources
        .common
        .repos
        .provider_connections
        .get_for_user(athlete.user_id, Some(athlete.tenant))
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.provider == "sciotte")
        .expect("the sciotte connection exists")
        .status
}

/// First sync: two rides the scraper vouches for land on Home, and the
/// athlete's last good sync is then aged past the freshness bands.
async fn synced_then_stale(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    scraper: &Scraper,
) -> DateTime<Utc> {
    let body = home(resources, athlete).await;
    assert_eq!(body["stale"], true, "never synced: {body}");
    await_background(resources).await;
    assert_eq!(ids(&home(resources, athlete).await), ["r2", "r1"]);
    let reads = scraper.list_reads();
    let last_good = Utc::now() - Duration::hours(5);
    age_last_sync(resources, athlete, last_good).await;
    assert_eq!(scraper.list_reads(), reads);
    last_good
}

/// A synced athlete, stale again, whose next refresh the scraper fails with
/// `failing`: Home has recorded the failure and says so. Returns the last
/// good sync.
async fn synced_then_failed(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    scraper: &Scraper,
    failing: ListScript,
) -> DateTime<Utc> {
    let last_good = synced_then_stale(resources, athlete, scraper).await;
    scraper.play_list(failing);
    let reads = scraper.list_reads();
    assert_eq!(home(resources, athlete).await["stale"], true);
    await_background(resources).await;
    assert_eq!(scraper.list_reads(), reads + 1);
    last_good
}

fn two_rides() -> Vec<ScrapedActivity> {
    vec![
        scraped_ride("r2", Utc::now() - Duration::days(1)),
        scraped_ride("r1", Utc::now() - Duration::days(3)),
    ]
}

/// Two rides and the day's new one on top.
fn three_rides() -> Vec<ScrapedActivity> {
    let mut rides = two_rides();
    rides.insert(0, scraped_ride("r3", Utc::now() - Duration::hours(2)));
    rides
}

// ---------------------------------------------------------------------------
// The list: only an answer the scraper vouched for is a sync
// ---------------------------------------------------------------------------

#[tokio::test]
#[serial]
async fn a_list_the_scraper_vouches_for_lands_on_home_and_is_fresh() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(two_rides())).await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-ok").await;

    let first = home(&resources, &athlete).await;
    assert_eq!(first["stale"], true, "{first}");
    assert!(first["sync_failure"].is_null(), "{first}");
    assert_eq!(first["activities"], json!([]));
    await_background(&resources).await;
    assert_eq!(scraper.list_reads(), 1);

    let body = home(&resources, &athlete).await;
    assert_eq!(body["stale"], false, "a vouched-for list is a sync: {body}");
    assert!(body["sync_failure"].is_null(), "{body}");
    assert_eq!(ids(&body), ["r2", "r1"]);
    assert_eq!(body["activities"][0]["provider"], "strava");
    assert!(instant(&body["as_of"]) > Utc::now() - Duration::minutes(1));
    await_background(&resources).await;
    assert_eq!(scraper.list_reads(), 1, "a fresh head reads no scraper");
}

/// Home's refresh reads the recent head — a window from thirty days back to
/// now — and nothing else: no per-row detail pass inside the list scrape,
/// which multiplies the browser time a scrape holds on the session, and no
/// closing `before` that would leave the head unread.
#[tokio::test]
#[serial]
async fn homes_refresh_reads_the_recent_head_without_a_detail_pass() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(two_rides())).await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-query").await;

    let asked_at = Utc::now();
    home(&resources, &athlete).await;
    await_background(&resources).await;

    let queries = scraper.list_queries();
    assert_eq!(queries.len(), 1, "{queries:?}");
    let params: Vec<(String, String)> = serde_urlencoded::from_str(&queries[0]).unwrap();
    let param = |key: &str| {
        params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, value)| value.clone())
    };
    let after = Utc
        .timestamp_opt(param("after").unwrap().parse().unwrap(), 0)
        .unwrap();
    assert!(
        (after - (asked_at - Duration::days(30)))
            .num_minutes()
            .abs()
            <= 2,
        "the head window opens thirty days back: {queries:?}"
    );
    assert!(
        param("before").is_none(),
        "the head window stays open: {queries:?}"
    );
    assert!(param("detail").is_none(), "no detail pass: {queries:?}");
}

#[tokio::test]
#[serial]
async fn an_empty_list_over_cached_rides_is_not_a_sync_and_is_paused_until_the_retry() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(two_rides())).await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-empty").await;
    // The scrape fails and answers `count: 0` as a success.
    let last_good =
        synced_then_failed(&resources, &athlete, &scraper, ListScript::Rows(Vec::new())).await;

    let body = home(&resources, &athlete).await;
    assert!(failed_at(&body) > last_good, "{body}");
    assert_eq!(body["sync_failure"]["provider"], "strava", "{body}");
    assert!(
        same_instant(instant(&body["sync_failure"]["last_synced_at"]), last_good),
        "the failure names the last good sync: {body}"
    );
    assert!(
        same_instant(instant(&body["as_of"]), last_good),
        "as_of stays the last good sync: {body}"
    );
    assert_eq!(ids(&body), ["r2", "r1"], "the cached rides stay on Home");
    assert_eq!(
        body["stale"], false,
        "a scraper that just failed is left alone for its pause: {body}"
    );
    await_background(&resources).await;
    assert_eq!(scraper.list_reads(), 2, "the next load reads no scraper");

    // The athlete retries once the scraper has recovered with the day's ride.
    scraper.play_list(ListScript::Rows(three_rides()));
    let retried = home_retry(&resources, &athlete).await;
    assert_eq!(
        retried["stale"], true,
        "a retry refreshes at once: {retried}"
    );
    await_background(&resources).await;
    assert_eq!(scraper.list_reads(), 3);
    let body = home(&resources, &athlete).await;
    assert_eq!(body["stale"], false, "{body}");
    assert!(
        body["sync_failure"].is_null(),
        "a good sync supersedes the failure: {body}"
    );
    assert_eq!(ids(&body), ["r3", "r2", "r1"]);
}

#[tokio::test]
#[serial]
async fn an_empty_list_over_an_empty_window_is_an_honest_zero() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(Vec::new())).await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-zero").await;

    home(&resources, &athlete).await;
    await_background(&resources).await;
    assert_eq!(scraper.list_reads(), 1);
    let body = home(&resources, &athlete).await;
    assert_eq!(
        body["stale"], false,
        "nothing cached, nothing to contradict: {body}"
    );
    assert!(body["sync_failure"].is_null(), "{body}");
    assert_eq!(body["activities"], json!([]));
}

/// The athlete's only ride of the month, synced, then stale: a Home load and
/// a retry are what the next three tests drive the scraper's answers through.
async fn synced_one_ride_then_stale(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    scraper: &Scraper,
) {
    home(resources, athlete).await;
    await_background(resources).await;
    assert_eq!(ids(&home(resources, athlete).await), ["solo"]);
    age_last_sync(resources, athlete, Utc::now() - Duration::hours(5)).await;
    scraper.play_list(ListScript::Rows(Vec::new()));
}

/// One list read the athlete's retry asks for, and what Home says
/// after it.
async fn after_a_retry(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    scraper: &Scraper,
) -> Value {
    let reads = scraper.list_reads();
    home_retry(resources, athlete).await;
    await_background(resources).await;
    assert_eq!(scraper.list_reads(), reads + 1, "the retry read the list");
    home(resources, athlete).await
}

async fn cached_solo_ride(resources: &ServerContext, athlete: &Athlete) -> Option<Activity> {
    resources
        .common
        .repos
        .activity_cache
        .get_cached_activity(athlete.user_id, &athlete.tenant, "sciotte", "solo")
        .await
        .unwrap()
}

/// An athlete who deleted the only ride of the month on Strava has no delete
/// signal on the mirror: the scraper just answers an empty list, which the
/// cache contradicts. Answered the same way three times across more than an
/// hour, the empty list is believed — the ride is evicted and Home stops
/// saying the sync failed — instead of failing for up to thirty days.
#[tokio::test]
#[serial]
async fn an_empty_list_answered_three_times_across_an_hour_is_believed_and_evicts_the_ride() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(vec![scraped_ride(
        "solo",
        Utc::now() - Duration::days(3),
    )]))
    .await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-deleted").await;
    synced_one_ride_then_stale(&resources, &athlete, &scraper).await;

    let first = after_a_retry(&resources, &athlete, &scraper).await;
    assert_eq!(first["sync_failure"]["provider"], "strava", "{first}");
    assert_eq!(ids(&first), ["solo"], "one empty answer is not believed");
    // The first empty answer was two hours ago.
    age_failure(&resources, &athlete, Utc::now() - Duration::hours(2)).await;

    let second = after_a_retry(&resources, &athlete, &scraper).await;
    assert!(second["sync_failure"].is_object(), "{second}");
    assert_eq!(ids(&second), ["solo"], "two empty answers are not believed");
    assert!(cached_solo_ride(&resources, &athlete).await.is_some());

    let third = after_a_retry(&resources, &athlete, &scraper).await;
    assert!(
        third["sync_failure"].is_null(),
        "the third empty answer, an hour after the first, is a sync: {third}"
    );
    assert_eq!(third["activities"], json!([]), "{third}");
    assert_eq!(third["stale"], false, "{third}");
    assert!(instant(&third["as_of"]) > Utc::now() - Duration::minutes(1));
    assert!(
        cached_solo_ride(&resources, &athlete).await.is_none(),
        "the deleted ride is evicted from the cache"
    );
    await_background(&resources).await;
    assert_eq!(scraper.list_reads(), 4, "a believed empty list is fresh");
}

/// Three empty answers inside the hour are the burst a failing scraper
/// produces, not a deletion: the ride stays and Home keeps saying the sync
/// failed.
#[tokio::test]
#[serial]
async fn three_empty_answers_inside_the_hour_stay_a_failed_sync() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(vec![scraped_ride(
        "solo",
        Utc::now() - Duration::days(3),
    )]))
    .await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-burst").await;
    synced_one_ride_then_stale(&resources, &athlete, &scraper).await;

    for _ in 0..3 {
        after_a_retry(&resources, &athlete, &scraper).await;
    }
    let body = home(&resources, &athlete).await;
    assert!(body["sync_failure"].is_object(), "{body}");
    assert_eq!(ids(&body), ["solo"], "{body}");
    assert!(cached_solo_ride(&resources, &athlete).await.is_some());
}

/// An empty answer after a different failure starts its own run: two failed
/// reads and one empty answer across two hours are not three empty answers.
#[tokio::test]
#[serial]
async fn an_empty_answer_after_failed_reads_starts_its_own_run() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(vec![scraped_ride(
        "solo",
        Utc::now() - Duration::days(3),
    )]))
    .await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-mixed").await;
    synced_one_ride_then_stale(&resources, &athlete, &scraper).await;

    scraper.play_list(ListScript::Fails);
    after_a_retry(&resources, &athlete, &scraper).await;
    age_failure(&resources, &athlete, Utc::now() - Duration::hours(2)).await;
    after_a_retry(&resources, &athlete, &scraper).await;
    scraper.play_list(ListScript::Rows(Vec::new()));
    let body = after_a_retry(&resources, &athlete, &scraper).await;
    assert!(body["sync_failure"].is_object(), "{body}");
    assert_eq!(ids(&body), ["solo"], "{body}");
    assert!(cached_solo_ride(&resources, &athlete).await.is_some());
}

#[tokio::test]
#[serial]
async fn a_failed_list_is_not_a_sync_and_home_says_when_it_failed() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(two_rides())).await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-fails").await;
    let before = Utc::now();
    let last_good = synced_then_failed(&resources, &athlete, &scraper, ListScript::Fails).await;

    let body = home(&resources, &athlete).await;
    let failed = failed_at(&body);
    assert!(before <= failed && failed <= Utc::now(), "{body}");
    assert!(same_instant(instant(&body["as_of"]), last_good));
    assert_eq!(ids(&body), ["r2", "r1"]);
}

#[tokio::test]
#[serial]
async fn a_list_missing_its_head_is_not_a_sync() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(two_rides())).await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-headless").await;
    let last_good = synced_then_failed(
        &resources,
        &athlete,
        &scraper,
        ListScript::Incomplete(three_rides()),
    )
    .await;

    let body = home(&resources, &athlete).await;
    assert!(failed_at(&body) > last_good, "{body}");
    assert_eq!(
        ids(&body),
        ["r2", "r1"],
        "a capture missing its head is not written through"
    );
}

#[tokio::test]
#[serial]
async fn a_list_whose_count_disagrees_with_its_rows_is_not_a_sync() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(two_rides())).await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-truncated").await;
    let last_good = synced_then_failed(
        &resources,
        &athlete,
        &scraper,
        ListScript::Truncated(vec![scraped_ride("r3", Utc::now() - Duration::hours(2))]),
    )
    .await;

    let body = home(&resources, &athlete).await;
    assert!(failed_at(&body) > last_good, "{body}");
    assert_eq!(ids(&body), ["r2", "r1"]);
}

// ---------------------------------------------------------------------------
// A failing scraper: paused, a longer pause each time, and the retry
// ---------------------------------------------------------------------------

/// Every Home load, each of its follow-ups and every stale-head turn used to
/// scrape a failing scraper again. The pause after a failure grows with the
/// failures in a row, and a load inside it reads nothing.
#[tokio::test]
#[serial]
async fn a_failing_scraper_is_paused_longer_after_each_failure() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(two_rides())).await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-backoff").await;
    synced_then_failed(&resources, &athlete, &scraper, ListScript::Fails).await;
    let reads = scraper.list_reads();

    // First failure: two minutes.
    age_failure(&resources, &athlete, Utc::now() - Duration::seconds(90)).await;
    assert_eq!(home(&resources, &athlete).await["stale"], false);
    age_failure(&resources, &athlete, Utc::now() - Duration::minutes(3)).await;
    assert_eq!(home(&resources, &athlete).await["stale"], true);
    await_background(&resources).await;
    assert_eq!(
        scraper.list_reads(),
        reads + 1,
        "read again once the pause is over"
    );

    // Second failure in a row: five minutes.
    age_failure(&resources, &athlete, Utc::now() - Duration::minutes(3)).await;
    let body = home(&resources, &athlete).await;
    assert_eq!(body["stale"], false, "the second pause is longer: {body}");
    await_background(&resources).await;
    assert_eq!(scraper.list_reads(), reads + 1);
    age_failure(&resources, &athlete, Utc::now() - Duration::minutes(6)).await;
    assert_eq!(home(&resources, &athlete).await["stale"], true);
    await_background(&resources).await;
    assert_eq!(scraper.list_reads(), reads + 2);
}

/// A chat turn — or the webhook, or a coach's group read — that fails to read
/// the head while Home's last sync is still fresh leaves Home saying the sync
/// failed. That provider is refreshed once the pause is over, and the
/// athlete's retry refreshes it at once, instead of the retry doing nothing
/// until the four-hour freshness band runs out.
#[tokio::test]
#[serial]
async fn a_failure_while_the_head_is_fresh_is_refreshed_and_the_retry_works() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(two_rides())).await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-fresh-fail").await;
    home(&resources, &athlete).await;
    await_background(&resources).await;
    // Synced ten minutes ago: well inside the freshness bands.
    age_last_sync(&resources, &athlete, Utc::now() - Duration::minutes(10)).await;

    scraper.play_list(ListScript::Fails);
    let turn = chat_activities(&resources, &athlete, Utc::now() - Duration::days(7)).await;
    assert_eq!(
        ids(&turn),
        ["r2", "r1"],
        "the coach is served the cached rides: {turn}"
    );
    let reads = scraper.list_reads();

    let body = home(&resources, &athlete).await;
    assert!(
        body["sync_failure"].is_object(),
        "Home says the chat's sync failed: {body}"
    );
    assert_eq!(body["stale"], false, "paused: {body}");
    await_background(&resources).await;
    assert_eq!(scraper.list_reads(), reads);

    // Once the pause is over the fresh head is refreshed all the same.
    age_failure(&resources, &athlete, Utc::now() - Duration::minutes(3)).await;
    assert_eq!(home(&resources, &athlete).await["stale"], true);
    await_background(&resources).await;
    assert_eq!(scraper.list_reads(), reads + 1);

    // Inside the new pause, the athlete's retry reaches the recovered scraper.
    scraper.play_list(ListScript::Rows(three_rides()));
    assert_eq!(home(&resources, &athlete).await["stale"], false);
    assert_eq!(home_retry(&resources, &athlete).await["stale"], true);
    await_background(&resources).await;
    let body = home(&resources, &athlete).await;
    assert!(body["sync_failure"].is_null(), "{body}");
    assert_eq!(ids(&body), ["r3", "r2", "r1"]);
}

// ---------------------------------------------------------------------------
// Who failed, and reads that do not cover the head
// ---------------------------------------------------------------------------

/// A healthy Garmin that synced a minute ago does not stand in for the
/// Strava refresh that failed: the failure names Strava and Strava's own last
/// good sync, while `as_of` is Garmin's.
#[tokio::test]
#[serial]
async fn the_failure_names_its_provider_and_that_providers_last_good_sync() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(two_rides())).await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-two").await;
    let last_good = synced_then_failed(&resources, &athlete, &scraper, ListScript::Fails).await;

    let repos = &resources.common.repos;
    repos
        .provider_connections
        .register_connection(
            athlete.user_id,
            athlete.tenant,
            "sciotte_garmin",
            &ConnectionType::Manual,
            None,
        )
        .await
        .unwrap();
    let swim = ActivityBuilder::new(
        "g1",
        "Natation",
        SportType::Swim,
        Utc::now() - Duration::hours(3),
        1_800,
        "garmin",
    )
    .build();
    repos
        .activity_cache
        .upsert_activities(athlete.user_id, &athlete.tenant, "sciotte_garmin", &[swim])
        .await
        .unwrap();
    let garmin_synced = Utc::now();
    repos
        .activity_cache
        .record_activity_fetch(
            athlete.user_id,
            &athlete.tenant,
            "sciotte_garmin",
            garmin_synced,
        )
        .await
        .unwrap();

    let body = home(&resources, &athlete).await;
    assert!(
        same_instant(instant(&body["as_of"]), garmin_synced),
        "{body}"
    );
    assert_eq!(body["sync_failure"]["provider"], "strava", "{body}");
    assert_eq!(body["sync_failure"]["provider_name"], "Strava", "{body}");
    assert!(
        same_instant(instant(&body["sync_failure"]["last_synced_at"]), last_good),
        "Strava's rows are as old as Strava's last good sync: {body}"
    );
}

/// A read of a closed window — a backfill of past weeks — lands its rows
/// without moving the head's freshness: Home still says the last refresh
/// failed, still dates the rows from the last good sync, and the head stays
/// due, instead of reading as synced for four hours while today's ride is
/// missing.
#[tokio::test]
#[serial]
async fn a_closed_window_read_moves_no_freshness_and_leaves_the_failure_standing() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(two_rides())).await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-season").await;
    let last_good = synced_then_failed(&resources, &athlete, &scraper, ListScript::Fails).await;
    let failed = failed_at(&home(&resources, &athlete).await);

    // A closed window inside the cache's retention, so the rows it lands are
    // kept: the weeks from sixty to forty days back.
    let season_start = Utc::now() - Duration::days(60);
    let season_end = Utc::now() - Duration::days(40);
    scraper.play_list(ListScript::Rows(vec![
        scraped_ride("s2", Utc::now() - Duration::days(45)),
        scraped_ride("s1", Utc::now() - Duration::days(55)),
    ]));
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let season = fetch_provider_head(
        &runtime,
        "sciotte",
        athlete.user_id,
        &athlete.tenant.to_string(),
        &ActivityQueryParams {
            after: Some(season_start.timestamp()),
            before: Some(season_end.timestamp()),
            limit: Some(200),
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(season.len(), 2);
    let cached = resources
        .common
        .repos
        .activity_cache
        .get_cached_activities(
            athlete.user_id,
            &athlete.tenant,
            Some("sciotte"),
            season_start,
            season_end,
            10,
        )
        .await
        .unwrap();
    assert_eq!(cached.len(), 2, "the season's rides are stored");

    let body = home(&resources, &athlete).await;
    assert!(same_instant(failed_at(&body), failed), "{body}");
    assert!(
        same_instant(instant(&body["as_of"]), last_good),
        "the season's rows are not a sync of today: {body}"
    );
}

/// An empty later page is the end of the list, not a failed sync: it is
/// neither recorded against the provider nor answered with the first page
/// again, and a later page served from the cache after a failed read is that
/// page.
#[tokio::test]
#[serial]
async fn an_empty_later_page_is_the_end_of_the_list_and_a_cached_page_is_that_page() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(two_rides())).await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-page").await;
    home(&resources, &athlete).await;
    await_background(&resources).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let tenant = athlete.tenant.to_string();
    let second_page = ActivityQueryParams {
        after: Some((Utc::now() - Duration::days(30)).timestamp()),
        before: None,
        limit: Some(1),
        offset: Some(1),
    };

    scraper.play_list(ListScript::Rows(Vec::new()));
    let page = fetch_provider_head(&runtime, "sciotte", athlete.user_id, &tenant, &second_page)
        .await
        .unwrap();
    assert!(page.is_empty());
    assert!(home(&resources, &athlete).await["sync_failure"].is_null());

    scraper.play_list(ListScript::Fails);
    let served =
        fetch_provider_activities(&runtime, "sciotte", athlete.user_id, &tenant, &second_page)
            .await
            .unwrap();
    let served: Vec<&str> = served.iter().map(Activity::id).collect();
    assert_eq!(
        served,
        ["r1"],
        "the cached second page, never the first again"
    );
    assert!(
        home(&resources, &athlete).await["sync_failure"].is_null(),
        "a later page says nothing about the head"
    );
}

// ---------------------------------------------------------------------------
// The chat's own read of the list
// ---------------------------------------------------------------------------

/// The coach asks "what did I do this week" while the scraper's page dies and
/// it answers `count: 0` as a success. The coach is served the week's cached
/// rides with a caveat, never "no activities"; the failure is recorded for
/// Home; and the false empty is never kept for the next ask.
#[tokio::test]
#[serial]
async fn a_chat_read_answered_empty_over_cached_rides_serves_them_and_records_the_failure() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(two_rides())).await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-chat").await;
    home(&resources, &athlete).await;
    await_background(&resources).await;

    scraper.play_list(ListScript::Rows(Vec::new()));
    let week = Utc::now() - Duration::days(7);
    let turn = chat_activities(&resources, &athlete, week).await;
    assert_eq!(ids(&turn), ["r2", "r1"], "{turn}");
    let caveat = turn["provider_unavailable"]["note"]
        .as_str()
        .unwrap_or_default();
    assert!(
        caveat.contains("may be missing"),
        "the coach is told the newest sessions may be missing: {turn}"
    );
    assert!(home(&resources, &athlete).await["sync_failure"].is_object());

    let reads = scraper.list_reads();
    let again = chat_activities(&resources, &athlete, week).await;
    assert_eq!(
        scraper.list_reads(),
        reads + 1,
        "the empty answer was not cached"
    );
    assert_eq!(ids(&again), ["r2", "r1"]);
}

// ---------------------------------------------------------------------------
// Reads that never answer: every bound gives up and says so
// ---------------------------------------------------------------------------

/// A refresh stuck on a scraper that never answers is given up at its bound,
/// recorded as a failed sync — Home says so — and frees its slot, so the
/// athlete's retry starts another read.
#[tokio::test]
#[serial]
async fn a_list_refresh_that_never_answers_is_recorded_failed_when_it_times_out() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(two_rides())).await;
    let _env = TestEnv::with(&[
        ("DRAVR_SCIOTTE_REMOTE_URL", url),
        ("PIERRE_REVALIDATION_TIMEOUT_SECS", "1".to_owned()),
    ]);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-hang").await;
    let last_good = synced_then_stale(&resources, &athlete, &scraper).await;

    scraper.play_list(ListScript::Hangs);
    assert_eq!(home(&resources, &athlete).await["stale"], true);
    await_background(&resources).await;
    let body = home(&resources, &athlete).await;
    assert!(failed_at(&body) > last_good, "{body}");
    assert_eq!(body["stale"], false, "{body}");

    let reads = scraper.list_reads();
    assert_eq!(home_retry(&resources, &athlete).await["stale"], true);
    await_background(&resources).await;
    assert_eq!(scraper.list_reads(), reads + 1, "the slot was released");
}

/// A refresh cut off after its sync had already landed — the read wrote its
/// rows and its mark, and the bound fired before it returned — reports the
/// sync that worked, never a failure recorded after it.
#[tokio::test]
#[serial]
async fn a_refresh_that_timed_out_after_its_sync_landed_is_not_recorded_failed() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(two_rides())).await;
    let _env = TestEnv::with(&[
        ("DRAVR_SCIOTTE_REMOTE_URL", url),
        ("PIERRE_REVALIDATION_TIMEOUT_SECS", "1".to_owned()),
    ]);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-late").await;
    synced_then_stale(&resources, &athlete, &scraper).await;

    scraper.play_list(ListScript::Hangs);
    let reads = scraper.list_reads();
    home(&resources, &athlete).await;
    scraper.await_list_reads(reads + 1).await;
    resources
        .common
        .repos
        .activity_cache
        .record_activity_fetch(athlete.user_id, &athlete.tenant, "sciotte", Utc::now())
        .await
        .unwrap();
    await_background(&resources).await;

    let body = home(&resources, &athlete).await;
    assert!(body["sync_failure"].is_null(), "{body}");
    assert_eq!(body["stale"], false, "{body}");
}

/// The nightly sweep's fetch that never answers is dropped at the sweep's
/// bound and recorded, so the athlete's Home opens on "sync failed" rather
/// than on rows that silently missed their sync.
#[tokio::test]
#[serial]
async fn a_sweep_fetch_dropped_at_its_bound_is_recorded_failed() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(two_rides())).await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-sweep").await;
    let last_good = synced_then_stale(&resources, &athlete, &scraper).await;

    scraper.play_list(ListScript::Hangs);
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let report = refresh_captures(
        &runtime,
        SweepBudget {
            per_connection: StdDuration::from_secs(1),
            ..SweepBudget::default()
        },
    )
    .await
    .unwrap();
    assert!(
        report
            .connections
            .iter()
            .any(|c| matches!(c.outcome, RefreshOutcome::Failed { .. })),
        "{report:?}"
    );

    let body = home(&resources, &athlete).await;
    assert!(failed_at(&body) > last_good, "{body}");
}

/// A flagged scrape session whose retried read answers nothing over cached
/// rides stays flagged: the page that should have held the list died, which
/// says nothing about whether the session is alive. A retry the scraper
/// vouches for re-arms it.
#[tokio::test]
#[serial]
async fn an_empty_answer_over_cached_rides_does_not_rearm_a_flagged_session() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(two_rides())).await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-flagged").await;
    synced_then_stale(&resources, &athlete, &scraper).await;
    let flag = || async {
        resources
            .common
            .repos
            .provider_connections
            .mark_needs_reauth(
                athlete.user_id,
                athlete.tenant,
                "sciotte",
                Some("session_expired"),
                Utc::now(),
            )
            .await
            .unwrap();
        exec_at(
            &resources,
            "UPDATE provider_connections SET status_changed_at = $1, reauth_retry_at = NULL \
             WHERE user_id = $2",
            Utc::now() - Duration::days(1),
            &athlete,
        )
        .await;
    };

    flag().await;
    scraper.play_list(ListScript::Rows(Vec::new()));
    let reads = scraper.list_reads();
    home(&resources, &athlete).await;
    await_background(&resources).await;
    assert_eq!(
        scraper.list_reads(),
        reads + 1,
        "the flagged session was retried"
    );
    assert_eq!(
        connection_status(&resources, &athlete).await,
        ConnectionStatus::NeedsReauth
    );

    flag().await;
    scraper.play_list(ListScript::Rows(three_rides()));
    home_retry(&resources, &athlete).await;
    await_background(&resources).await;
    assert_eq!(
        connection_status(&resources, &athlete).await,
        ConnectionStatus::Active
    );
}

// ---------------------------------------------------------------------------
// The route: only a route without coordinates is no GPS
// ---------------------------------------------------------------------------

/// An athlete with the two rides synced, and the scraper scripted to answer
/// their detail reads with `detail`.
async fn synced_for_routes(
    label: &str,
    detail: DetailScript,
    bounds: &[(&'static str, String)],
) -> (Arc<ServerContext>, Athlete, Arc<Scraper>, TestEnv) {
    let (url, scraper) = spawn_scraper(ListScript::Rows(two_rides())).await;
    let mut vars = vec![("DRAVR_SCIOTTE_REMOTE_URL", url)];
    vars.extend_from_slice(bounds);
    let env = TestEnv::with(&vars);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, label).await;
    home(&resources, &athlete).await;
    await_background(&resources).await;
    scraper.play_detail(detail);
    (resources, athlete, scraper, env)
}

#[tokio::test]
#[serial]
async fn a_detail_with_a_route_draws_the_map_and_is_stored() {
    let (resources, athlete, scraper, _env) =
        synced_for_routes("route-drawn", DetailScript::Route, &[]).await;

    let body = route(&resources, &athlete, "r2").await;
    assert!(body["reason"].is_null(), "{body}");
    let points = body["route"]["coordinates"].as_array().unwrap();
    assert!(points.len() > 2, "{body}");
    assert_eq!(body["route"]["source_tool"], "strava");
    assert_eq!(scraper.detail_reads(), 1);
    assert!(matches!(
        stored_route(&resources, &athlete, "r2").await,
        Some(StoredRouteTrack::Drawn { ref source, .. }) if source == "streams"
    ));

    assert_eq!(route(&resources, &athlete, "r2").await, body);
    assert_eq!(scraper.detail_reads(), 1, "a drawn route is read once");
    assert!(has_gps(&home(&resources, &athlete).await, "r2"));
}

/// The scraper leaves `route` out when no read settled the page's route; the
/// platform answers that `unavailable` and reads it again, never `no_gps`.
#[tokio::test]
#[serial]
async fn a_detail_without_a_route_is_unavailable_never_no_gps_and_is_read_again() {
    let (resources, athlete, scraper, _env) =
        synced_for_routes("route-missing", DetailScript::NoRoute, &[]).await;

    let before = Utc::now();
    let body = route(&resources, &athlete, "r2").await;
    assert_eq!(body, json!({ "route": null, "reason": "unavailable" }));
    assert_eq!(scraper.detail_reads(), 1);
    let Some(StoredRouteTrack::Unavailable {
        reason,
        expires_at: Some(expires_at),
        ..
    }) = stored_route(&resources, &athlete, "r2").await
    else {
        panic!("an unread route is stored with an expiry");
    };
    assert_eq!(reason, "unavailable");
    let recheck = Duration::minutes(UNREAD_ROUTE_RECHECK_MINUTES);
    assert!(before + recheck <= expires_at && expires_at <= Utc::now() + recheck);
    assert!(
        has_gps(&home(&resources, &athlete).await, "r2"),
        "a read that settled nothing never says the ride has no GPS"
    );

    // Asked again at once without a retry: the stored answer, no scraper.
    assert_eq!(route(&resources, &athlete, "r2").await, body);
    assert_eq!(scraper.detail_reads(), 1);

    // Once it expires the route is read again, and this time it draws.
    scraper.play_detail(DetailScript::Route);
    expire_route_reads(&resources, &athlete).await;
    let body = route(&resources, &athlete, "r2").await;
    assert_eq!(scraper.detail_reads(), 2);
    assert!(
        body["route"]["coordinates"].as_array().unwrap().len() > 2,
        "{body}"
    );
}

/// The map's retry reaches the scraper at once, past the stored
/// `unavailable`, and draws the route the recovered scraper reads.
#[tokio::test]
#[serial]
async fn a_retry_of_an_unavailable_route_reaches_the_scraper_again() {
    let (resources, athlete, scraper, _env) =
        synced_for_routes("route-retry", DetailScript::Fails, &[]).await;

    let body = route(&resources, &athlete, "r2").await;
    assert_eq!(body, json!({ "route": null, "reason": "unavailable" }));
    let reads = scraper.detail_reads();

    scraper.play_detail(DetailScript::Route);
    let retried = route_retry(&resources, &athlete, "r2").await;
    assert_eq!(
        scraper.detail_reads(),
        reads + 1,
        "the retry read the scraper"
    );
    assert!(
        retried["route"]["coordinates"].as_array().unwrap().len() > 2,
        "{retried}"
    );
    assert!(matches!(
        stored_route(&resources, &athlete, "r2").await,
        Some(StoredRouteTrack::Drawn { .. })
    ));
}

#[tokio::test]
#[serial]
async fn a_failed_detail_read_is_unavailable_not_an_error() {
    let (resources, athlete, scraper, _env) =
        synced_for_routes("route-fails", DetailScript::Fails, &[]).await;

    let body = route(&resources, &athlete, "r2").await;
    assert_eq!(body, json!({ "route": null, "reason": "unavailable" }));
    assert!(scraper.detail_reads() >= 1);
    assert!(has_gps(&home(&resources, &athlete).await, "r2"));
    assert!(matches!(
        stored_route(&resources, &athlete, "r2").await,
        Some(StoredRouteTrack::Unavailable { ref reason, expires_at: Some(_), .. })
            if reason == "unavailable"
    ));
}

/// The contract with the scraper for "read the page, the ride recorded no
/// GPS": a `route` holding no coordinates, exactly as the scraper's own
/// serializer writes an empty `RouteTrack`. It settles `no_gps` for good.
#[tokio::test]
#[serial]
async fn an_indoor_ride_as_sciotte_serializes_it_settles_no_gps_and_is_never_read_again() {
    let wire = serde_json::to_value(scraped_route(Vec::new())).unwrap();
    assert_eq!(
        wire,
        json!({ "coordinates": [] }),
        "the scraper's no-GPS answer"
    );
    let (resources, athlete, scraper, _env) =
        synced_for_routes("route-no-gps", DetailScript::EmptyRoute, &[]).await;

    let body = route(&resources, &athlete, "r2").await;
    assert_eq!(body, json!({ "route": null, "reason": "no_gps" }));
    assert_eq!(scraper.detail_reads(), 1);
    assert!(matches!(
        stored_route(&resources, &athlete, "r2").await,
        Some(StoredRouteTrack::Unavailable { ref reason, expires_at: None, .. })
            if reason == "no_gps"
    ));
    assert!(
        !has_gps(&home(&resources, &athlete).await, "r2"),
        "a route read that found no coordinates settles has_gps"
    );
    expire_route_reads(&resources, &athlete).await;
    assert_eq!(route(&resources, &athlete, "r2").await, body);
    assert_eq!(route_retry(&resources, &athlete, "r2").await, body);
    assert_eq!(
        scraper.detail_reads(),
        1,
        "a settled no_gps is never read again, not even on a retry"
    );
}

// ---------------------------------------------------------------------------
// The route read's bounds
// ---------------------------------------------------------------------------

/// A detail read slower than the request's bound answers `unavailable` and
/// stores nothing then; the read keeps going, stores what the scraper says,
/// and the next request draws it without reading again.
#[tokio::test]
#[serial]
async fn a_route_read_past_its_answer_bound_stores_what_the_scraper_says_later() {
    let (resources, athlete, scraper, _env) = synced_for_routes(
        "route-slow",
        DetailScript::SlowRoute(2_000),
        &[("PIERRE_HOME_ROUTE_ANSWER_SECS", "1".to_owned())],
    )
    .await;

    let started = Instant::now();
    let body = route(&resources, &athlete, "r2").await;
    assert!(
        started.elapsed() < StdDuration::from_millis(1_900),
        "answered at its bound"
    );
    assert_eq!(body, json!({ "route": null, "reason": "unavailable" }));
    assert!(
        stored_route(&resources, &athlete, "r2").await.is_none(),
        "a read still running is not a failed one"
    );

    await_background(&resources).await;
    assert!(matches!(
        stored_route(&resources, &athlete, "r2").await,
        Some(StoredRouteTrack::Drawn { .. })
    ));
    let body = route(&resources, &athlete, "r2").await;
    assert!(body["route"].is_object(), "{body}");
    assert_eq!(scraper.detail_reads(), 1, "one read between them");
}

/// Two route requests at once for one athlete take turns at the scraper. The
/// one queued behind a slow read answers `unavailable` at its bound without
/// reading or storing anything, and the slow read is never cut short: it
/// lands, and the scraper is only ever asked one read at a time.
#[tokio::test]
#[serial]
async fn a_route_read_queued_behind_a_slow_one_answers_unavailable_and_stores_nothing() {
    let (resources, athlete, scraper, _env) = synced_for_routes(
        "route-queued",
        DetailScript::SlowRoute(2_000),
        &[("PIERRE_HOME_ROUTE_ANSWER_SECS", "1".to_owned())],
    )
    .await;

    let (first, second) = tokio::join!(
        route(&resources, &athlete, "r2"),
        route(&resources, &athlete, "r1"),
    );
    let unavailable = json!({ "route": null, "reason": "unavailable" });
    assert_eq!(first, unavailable);
    assert_eq!(second, unavailable);
    await_background(&resources).await;
    assert_eq!(scraper.detail_reads(), 1, "the queued request never read");
    let stored = [
        stored_route(&resources, &athlete, "r2").await,
        stored_route(&resources, &athlete, "r1").await,
    ];
    assert_eq!(
        stored.iter().flatten().count(),
        1,
        "the read that ran is stored, the one that never ran is not"
    );
    assert!(
        stored
            .iter()
            .flatten()
            .all(|s| matches!(s, StoredRouteTrack::Drawn { .. })),
        "the slow read landed a route, not a failure"
    );
}

/// A detail read that never answers is given up at the provider bound and
/// stored `unavailable`; the request itself answered at its own bound.
#[tokio::test]
#[serial]
async fn a_detail_read_that_never_answers_is_given_up_and_stored_unavailable() {
    let (resources, athlete, _scraper, _env) = synced_for_routes(
        "route-hang",
        DetailScript::Hangs,
        &[
            ("PIERRE_HOME_ROUTE_ANSWER_SECS", "1".to_owned()),
            ("PIERRE_HOME_ROUTE_PROVIDER_READ_SECS", "2".to_owned()),
        ],
    )
    .await;

    let started = Instant::now();
    let body = route(&resources, &athlete, "r2").await;
    assert!(started.elapsed() < StdDuration::from_millis(1_900));
    assert_eq!(body, json!({ "route": null, "reason": "unavailable" }));
    assert!(stored_route(&resources, &athlete, "r2").await.is_none());

    await_background(&resources).await;
    assert!(matches!(
        stored_route(&resources, &athlete, "r2").await,
        Some(StoredRouteTrack::Unavailable { ref reason, expires_at: Some(_), .. })
            if reason == "unavailable"
    ));
    assert!(has_gps(&home(&resources, &athlete).await, "r2"));
}

// ---------------------------------------------------------------------------
// The 2026-09-29 incident: a list and a route read at once on one session
// ---------------------------------------------------------------------------

/// A synced athlete, stale again, loads Home: the refresh starts a list walk,
/// and the page reads the latest ride's route while the walk is still on the
/// same scrape session. The detail read finishes and closes the browser the
/// walk shares; the walk's page dies and the scraper answers as `dead` says.
///
/// Whatever that answer, Home keeps the rides, dates them from the last good
/// sync, says Strava's sync failed, and draws the map the detail read landed;
/// the athlete's retry, once the scraper is well again, lands the day's ride.
async fn the_incident(label: &str, dead: DeadPage) {
    let (url, scraper) = spawn_scraper(ListScript::Rows(two_rides())).await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, label).await;
    let last_good = synced_then_stale(&resources, &athlete, &scraper).await;
    assert_eq!(scraper.detail_reads(), 0, "no route read yet");

    scraper.play_list(ListScript::DiesUnderADetail(dead));
    let reads = scraper.list_reads();
    let loaded = home(&resources, &athlete).await;
    assert_eq!(
        loaded["stale"], true,
        "the stale head is refreshed: {loaded}"
    );
    scraper.await_list_reads(reads + 1).await;
    // The page reads the latest ride's map while the walk is in flight.
    let map = route(&resources, &athlete, "r2").await;
    await_background(&resources).await;
    assert_eq!(
        scraper.max_in_flight(),
        2,
        "the list walk and the route read overlapped on the session"
    );

    assert!(
        map["route"]["coordinates"].as_array().unwrap().len() > 2,
        "the route read that finished first draws the map: {map}"
    );
    let body = home(&resources, &athlete).await;
    assert_failed_sync_kept_the_rides(&body, last_good);
    assert!(has_gps(&body, "r2"));
    assert_eq!(body["stale"], false, "paused after the failure: {body}");

    retry_lands_the_days_ride(&resources, &athlete, &scraper).await;
}

/// Home says Strava's sync failed after `last_good`, dates the rides from
/// `last_good`, and keeps them.
fn assert_failed_sync_kept_the_rides(body: &Value, last_good: DateTime<Utc>) {
    assert_eq!(body["sync_failure"]["provider"], "strava", "{body}");
    assert!(failed_at(body) > last_good, "{body}");
    assert!(
        same_instant(instant(&body["sync_failure"]["last_synced_at"]), last_good),
        "{body}"
    );
    assert!(
        same_instant(instant(&body["as_of"]), last_good),
        "a failed read is not a sync: {body}"
    );
    assert_eq!(ids(body), ["r2", "r1"], "the rides stay on Home");
}

/// The scraper is well again with the day's ride: the athlete's retry reads
/// it at once and lands it on top, and the failure is gone.
async fn retry_lands_the_days_ride(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    scraper: &Scraper,
) {
    scraper.play_list(ListScript::Rows(three_rides()));
    assert_eq!(home_retry(resources, athlete).await["stale"], true);
    await_background(resources).await;
    let body = home(resources, athlete).await;
    assert!(body["sync_failure"].is_null(), "{body}");
    assert_eq!(ids(&body), ["r3", "r2", "r1"]);
    assert!(instant(&body["as_of"]) > Utc::now() - Duration::minutes(1));
}

/// The incident as it happened: the dead walk answered `200` with
/// `count: 0` and `head_complete: true`, and the platform recorded a fresh
/// sync that left Home empty for four hours.
#[tokio::test]
#[serial]
async fn a_list_walk_killed_by_a_concurrent_route_read_answering_empty_is_a_failed_sync() {
    the_incident("incident-empty", DeadPage::AnsweredEmpty).await;
}

/// The same overlap against a scraper that answers a walk that read nothing
/// with its error status.
#[tokio::test]
#[serial]
async fn a_list_walk_killed_by_a_concurrent_route_read_answering_an_error_is_a_failed_sync() {
    the_incident("incident-error", DeadPage::AnsweredError).await;
}

// ---------------------------------------------------------------------------
// The scraper's error answers, as it sends them
// ---------------------------------------------------------------------------

/// A scraper that sheds the read (`503 scraper_busy` with its `Retry-After`)
/// read nothing: a failed sync, the rides kept, and no reconnect asked of the
/// athlete, whose session is fine.
#[tokio::test]
#[serial]
async fn a_scraper_shedding_the_read_is_a_failed_sync_not_a_reconnect() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(two_rides())).await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-busy").await;
    let last_good = synced_then_failed(&resources, &athlete, &scraper, ListScript::Busy).await;

    assert_failed_sync_kept_the_rides(&home(&resources, &athlete).await, last_good);
    assert_eq!(
        connection_status(&resources, &athlete).await,
        ConnectionStatus::Active,
        "a busy scraper says nothing about the session"
    );
    retry_lands_the_days_ride(&resources, &athlete, &scraper).await;
}

/// A session the provider refused (`401 session_expired`) is a reconnect the
/// athlete has to make: the connection is flagged, the rides stay, and the
/// refusal is never recorded as a fresh sync.
#[tokio::test]
#[serial]
async fn a_session_the_provider_refused_flags_a_reconnect_and_is_never_a_sync() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(two_rides())).await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-expired").await;
    let last_good =
        synced_then_failed(&resources, &athlete, &scraper, ListScript::SessionExpired).await;

    assert_eq!(
        connection_status(&resources, &athlete).await,
        ConnectionStatus::NeedsReauth,
        "the athlete is asked to reconnect"
    );
    let reads = scraper.list_reads();
    let body = home(&resources, &athlete).await;
    assert!(
        same_instant(instant(&body["as_of"]), last_good),
        "a refused session is not a sync: {body}"
    );
    assert_eq!(ids(&body), ["r2", "r1"]);
    assert!(
        body["sync_failure"].is_null(),
        "a dead session is a reconnect, not a failed sync: {body}"
    );
    await_background(&resources).await;
    assert_eq!(
        scraper.list_reads(),
        reads,
        "a Home load does not scrape a session the provider refused"
    );
}

/// The scraper instance that took the session's import went down and the
/// read reached its replacement, which holds no session (`401
/// session_not_found`). The platform imports the session again and re-sends
/// the read: the rides land, and the athlete is never sent to reconnect.
#[tokio::test]
#[serial]
async fn a_session_the_scraper_lost_after_its_import_is_imported_again_not_a_reconnect() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(two_rides())).await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-lost").await;
    let last_good = synced_then_stale(&resources, &athlete, &scraper).await;

    scraper.play_list(ListScript::Rows(three_rides()));
    scraper.forget_next_import.store(true, Ordering::SeqCst);
    let imports = scraper.imports();
    let reads = scraper.list_reads();
    home(&resources, &athlete).await;
    await_background(&resources).await;
    assert_eq!(
        scraper.imports(),
        imports + 2,
        "imported again after the 401"
    );
    assert_eq!(scraper.list_reads(), reads + 2, "the read was re-sent once");

    let body = home(&resources, &athlete).await;
    assert!(body["sync_failure"].is_null(), "{body}");
    assert_eq!(ids(&body), ["r3", "r2", "r1"]);
    assert!(instant(&body["as_of"]) > last_good);
    assert_eq!(
        connection_status(&resources, &athlete).await,
        ConnectionStatus::Active
    );
}

/// The scraper goes down: while it refuses connections, Home's refresh is a
/// failed sync and a route read answers `unavailable`, never `no_gps`. It
/// comes back holding no session; the athlete's retries import the session
/// again, land the day's ride and draw the map.
#[tokio::test]
#[serial]
async fn a_restarted_scraper_is_a_failed_sync_while_down_and_the_retries_land_once_it_is_back() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(two_rides())).await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "sync-restart").await;
    let last_good = synced_then_stale(&resources, &athlete, &scraper).await;

    scraper.stop().await;
    assert_eq!(home(&resources, &athlete).await["stale"], true);
    await_background(&resources).await;
    assert_failed_sync_kept_the_rides(&home(&resources, &athlete).await, last_good);
    let map = route(&resources, &athlete, "r2").await;
    assert_eq!(map, json!({ "route": null, "reason": "unavailable" }));
    assert!(has_gps(&home(&resources, &athlete).await, "r2"));
    assert_eq!(
        connection_status(&resources, &athlete).await,
        ConnectionStatus::Active,
        "a scraper that is down says nothing about the session"
    );

    scraper.restart().await;
    let imports = scraper.imports();
    retry_lands_the_days_ride(&resources, &athlete, &scraper).await;
    assert!(
        scraper.imports() > imports,
        "the session was imported again"
    );

    let map = route_retry(&resources, &athlete, "r2").await;
    assert!(
        map["route"]["coordinates"].as_array().unwrap().len() > 2,
        "{map}"
    );
}

// ---------------------------------------------------------------------------
// Many route reads at once
// ---------------------------------------------------------------------------

/// Five rides, newest first.
fn five_rides() -> Vec<ScrapedActivity> {
    (1..=5)
        .rev()
        .map(|n| scraped_ride(&format!("f{n}"), Utc::now() - Duration::days(6 - n)))
        .collect()
}

/// Five route reads at once for one athlete — five different rides, then five
/// asks for one ride — take turns on the scrape session: never two detail
/// reads in flight, every map drawn, and one ride read once however many
/// asked for it.
#[tokio::test]
#[serial]
async fn route_reads_at_once_take_turns_on_the_session_and_every_map_draws() {
    let (url, scraper) = spawn_scraper(ListScript::Rows(five_rides())).await;
    let _env = TestEnv::point_at(url);
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_on_the_mirror(&resources, "route-many").await;
    home(&resources, &athlete).await;
    await_background(&resources).await;
    scraper.play_detail(DetailScript::SlowRoute(300));

    let rides = ["f5", "f4", "f3", "f2", "f1"];
    let maps = join_all(rides.iter().map(|id| route(&resources, &athlete, id))).await;
    for (id, map) in rides.iter().zip(&maps) {
        assert!(
            map["route"]["coordinates"].as_array().unwrap().len() > 2,
            "{id}: {map}"
        );
    }
    assert_eq!(scraper.detail_reads(), 5);
    assert_eq!(
        scraper.max_details_in_flight(),
        1,
        "one detail read on the session at a time"
    );

    // A sixth ride lands on the next sync; five asks for its map at once.
    let before = scraper.detail_reads();
    let sixth = scraped_ride("f6", Utc::now() - Duration::hours(1));
    scraper.play_list(ListScript::Rows(once(sixth).chain(five_rides()).collect()));
    age_last_sync(&resources, &athlete, Utc::now() - Duration::hours(5)).await;
    assert_eq!(home(&resources, &athlete).await["stale"], true);
    await_background(&resources).await;
    let asks = join_all((0..5).map(|_| route(&resources, &athlete, "f6"))).await;
    assert!(
        asks.iter()
            .all(|map| *map == asks[0] && map["route"].is_object()),
        "{asks:?}"
    );
    assert_eq!(scraper.detail_reads(), before + 1, "one ride, one read");
    assert_eq!(scraper.max_details_in_flight(), 1);
}
