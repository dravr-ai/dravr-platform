// ABOUTME: carnet#582 — the one-time walk of a Strava athlete's history seeds their all-time bests before any record is told
// ABOUTME: Drives the real seed pass and StravaProvider against a Strava-shaped mock serving per-athlete histories and streams
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Personal-best seed suite (carnet#582).
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
//
// A personal record is announced only against the athlete's whole running
// history, so each Strava athlete's history is walked once, paced by the
// server's shared Strava budget, before any record is told. These tests point
// the real Strava provider at a local mock (`PIERRE_STRAVA_API_BASE_URL`, the
// registry seam) that serves each athlete's history by their access token,
// pages it at a few activities so a short history spans several pages, and
// counts every listing and every streams request.
#![cfg(all(
    feature = "health-sync",
    feature = "provider-strava",
    feature = "client-notifications"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::env;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{TimeZone, Utc};
use pierre_core::models::{Activity, ActivityBuilder, SportType, TenantId, UserOAuthToken};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::personal_best_seed::{
    run_personal_best_seed_pass, strava_provider, SeedPassReport,
};
use pierre_notifications::events::event_params;
use pierre_notifications::TenantId as CommereTenantId;
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::request_budget::{
    budget_counter_key, budget_period, ProviderRateLimiter, RateLimitStatus, ONE_DAY,
    PLATFORM_SCOPE,
};
use pierre_services::personal_bests::athlete_lease_name;
use serde_json::{json, Value};
use serial_test::serial;
use tokio::net::TcpListener;
use tokio::time::sleep;
use uuid::Uuid;

/// The most activities the mock returns per listing, whatever `per_page`
/// asks for, so a history of a few activities spans several pages.
const PAGE_CAP: usize = 3;

/// Seconds in a day.
const DAY: i64 = 86_400;

/// One activity of a mocked history.
#[derive(Debug, Clone, Copy)]
struct MockActivity {
    id: u64,
    sport: &'static str,
    start: i64,
    seconds_per_km: u32,
    km: u32,
}

/// A 6 km run at `seconds_per_km`, `days_ago` days before now: its 5 km best
/// effort is exactly `5 × seconds_per_km`.
fn run(id: u64, days_ago: i64, seconds_per_km: u32) -> MockActivity {
    MockActivity {
        id,
        sport: "Run",
        start: Utc::now().timestamp() - days_ago * DAY,
        seconds_per_km,
        km: 6,
    }
}

/// A 20 km ride `days_ago` days before now: never measured.
fn ride(id: u64, days_ago: i64) -> MockActivity {
    MockActivity {
        id,
        sport: "Ride",
        start: Utc::now().timestamp() - days_ago * DAY,
        seconds_per_km: 120,
        km: 20,
    }
}

/// What the Strava-shaped mock serves and saw.
#[derive(Default)]
struct MockStrava {
    /// Each athlete's history, by the access token that reads it.
    histories: Mutex<HashMap<String, Vec<MockActivity>>>,
    /// Listing requests, by access token.
    list_hits: Mutex<HashMap<String, usize>>,
    /// Streams requests, by activity id.
    streams_hits: Mutex<HashMap<u64, usize>>,
    /// Activities whose streams request answers 500.
    failing: Mutex<HashSet<u64>>,
    /// How long a streams request takes to answer.
    streams_delay: Mutex<Duration>,
    /// Streams requests being answered right now, by access token.
    in_flight: Mutex<HashMap<String, usize>>,
    /// The most streams requests one token ever had in flight at once.
    max_in_flight: Mutex<HashMap<String, usize>>,
}

impl MockStrava {
    fn activity(&self, id: u64) -> Option<MockActivity> {
        self.histories
            .lock()
            .unwrap()
            .values()
            .flatten()
            .find(|activity| activity.id == id)
            .copied()
    }

    /// An activity the athlete reading with `token` uploads now.
    fn upload(&self, token: &str, activity: MockActivity) {
        self.histories
            .lock()
            .unwrap()
            .entry(token.to_owned())
            .or_default()
            .push(activity);
    }

    fn list_hits(&self, token: &str) -> usize {
        self.list_hits
            .lock()
            .unwrap()
            .get(token)
            .copied()
            .unwrap_or(0)
    }

    fn streams_hits(&self, id: u64) -> usize {
        self.streams_hits
            .lock()
            .unwrap()
            .get(&id)
            .copied()
            .unwrap_or(0)
    }

    fn total_streams_hits(&self) -> usize {
        self.streams_hits.lock().unwrap().values().sum()
    }

    /// Every request the mock answered: listings and streams, all tokens.
    fn total_requests(&self) -> usize {
        self.list_hits.lock().unwrap().values().sum::<usize>() + self.total_streams_hits()
    }

    fn max_in_flight(&self, token: &str) -> usize {
        self.max_in_flight
            .lock()
            .unwrap()
            .get(token)
            .copied()
            .unwrap_or(0)
    }
}

/// The access token a request carries.
fn bearer(headers: &HeaderMap) -> String {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default()
        .to_owned()
}

/// `GET /athlete/activities`: the reader's activities that started before
/// `before`, newest first, at most [`PAGE_CAP`] of them.
async fn list_activities(
    State(mock): State<Arc<MockStrava>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Value> {
    let token = bearer(&headers);
    *mock
        .list_hits
        .lock()
        .unwrap()
        .entry(token.clone())
        .or_default() += 1;
    let before: i64 = query
        .get("before")
        .and_then(|value| value.parse().ok())
        .unwrap_or(i64::MAX);
    let per_page: usize = query
        .get("per_page")
        .and_then(|value| value.parse().ok())
        .unwrap_or(30);
    let mut listed: Vec<MockActivity> = mock
        .histories
        .lock()
        .unwrap()
        .get(&token)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|activity| activity.start < before)
        .collect();
    listed.sort_by_key(|activity| Reverse(activity.start));
    listed.truncate(per_page.min(PAGE_CAP));
    Json(Value::Array(
        listed
            .iter()
            .map(|activity| {
                json!({
                    "id": activity.id,
                    "name": "Morning activity",
                    "type": activity.sport,
                    "sport_type": activity.sport,
                    "start_date": Utc.timestamp_opt(activity.start, 0).unwrap().to_rfc3339(),
                    "distance": f64::from(activity.km) * 1000.0,
                    "elapsed_time": activity.seconds_per_km * activity.km,
                    "total_elevation_gain": 10.0
                })
            })
            .collect(),
    ))
}

/// `GET /activities/{id}/streams`: one sample a second with the cumulative
/// distance, as Strava sends a run recorded by a watch.
async fn activity_streams(
    State(mock): State<Arc<MockStrava>>,
    headers: HeaderMap,
    Path(id): Path<u64>,
) -> Result<Json<Value>, StatusCode> {
    *mock.streams_hits.lock().unwrap().entry(id).or_default() += 1;
    let token = bearer(&headers);
    {
        let mut in_flight = mock.in_flight.lock().unwrap();
        let now = in_flight.entry(token.clone()).or_default();
        *now += 1;
        let mut max = mock.max_in_flight.lock().unwrap();
        let most = max.entry(token.clone()).or_default();
        *most = (*most).max(*now);
    }
    let delay = *mock.streams_delay.lock().unwrap();
    sleep(delay).await;
    *mock.in_flight.lock().unwrap().entry(token).or_default() -= 1;
    if mock.failing.lock().unwrap().contains(&id) {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let activity = mock
        .activity(id)
        .expect("streams asked for an activity the mock serves");
    let duration = activity.seconds_per_km * activity.km;
    let time: Vec<u32> = (0..=duration).collect();
    let distance: Vec<f64> = time
        .iter()
        .map(|t| f64::from(*t) * 1000.0 / f64::from(activity.seconds_per_km))
        .collect();
    Ok(Json(json!({
        "time": { "data": time },
        "distance": { "data": distance }
    })))
}

async fn mock_strava() -> (String, Arc<MockStrava>) {
    let mock = Arc::new(MockStrava::default());
    let app = Router::new()
        .route("/athlete/activities", get(list_activities))
        .route("/activities/{id}/streams", get(activity_streams))
        .with_state(Arc::clone(&mock));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (format!("http://{addr}"), mock)
}

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

/// A server context whose Strava provider talks to `api_base`.
///
/// The registry reads `PIERRE_STRAVA_API_BASE_URL` when the context is
/// built, so the guard must be alive before this call.
async fn context_pointed_at(api_base: &str, per_window: u32) -> (Arc<ServerContext>, EnvGuard) {
    let guard = EnvGuard::set(&[
        ("PIERRE_STRAVA_API_BASE_URL", api_base.to_owned()),
        ("STRAVA_CLIENT_ID", "test_client".to_owned()),
        ("STRAVA_CLIENT_SECRET", "test_secret".to_owned()),
    ]);
    let resources = common::create_test_server_resources().await.unwrap();
    (with_strava_window(&resources, per_window), guard)
}

/// `resources` composed as the server composes its registry, with a Strava
/// window allowing `per_window` requests a day for each signing app, a
/// quarter of which the walk may take. The registry is built here, after the
/// environment pointing Strava at the mock is set.
fn with_strava_window(resources: &Arc<ServerContext>, per_window: u32) -> Arc<ServerContext> {
    let limiter = ProviderRateLimiter::new(
        Arc::clone(&resources.common.repos.usage_counters),
        Arc::clone(&resources.common.repos.provider_connections),
    );
    limiter.set_budgets("strava", &[(per_window, ONE_DAY)]);
    let mut context = (**resources).clone();
    context.fitness.provider_registry =
        Arc::new(ProviderRegistry::new().with_request_limiter(Arc::new(limiter)));
    Arc::new(context)
}

/// An OAuth-connected Strava athlete.
struct Athlete {
    user_id: Uuid,
    tenant_id: TenantId,
    token: String,
}

/// Connect an athlete to Strava with `history` as what Strava holds for them.
async fn linked_athlete(
    resources: &ServerContext,
    mock: &MockStrava,
    email: &str,
    owner_id: u64,
    history: Vec<MockActivity>,
) -> Athlete {
    let (user_id, _user, tenant_id) =
        common::create_test_user_with_plan(&resources.agent.database, email, "starter")
            .await
            .unwrap();
    // >= 40 chars and not "at_"-prefixed: the provider's token validation.
    let token = format!("strava_access_token_{owner_id:0>30}");
    let now = Utc::now();
    resources
        .common
        .repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id: tenant_id.to_string(),
            provider: "strava".to_owned(),
            access_token: token.clone(),
            refresh_token: Some("strava_refresh".to_owned()),
            token_type: "Bearer".to_owned(),
            expires_at: Some(now + chrono::Duration::hours(6)),
            scope: Some("activity:read_all".to_owned()),
            provider_user_id: Some(owner_id.to_string()),
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    mock.histories
        .lock()
        .unwrap()
        .insert(token.clone(), history);
    Athlete {
        user_id,
        tenant_id,
        token,
    }
}

/// Open a fresh Strava window: the windows are counted in the database, so
/// opening one clears the counted buckets. The window is a day long, so a
/// test never straddles one of its boundaries the way it could a 15-minute
/// one.
async fn open_window(resources: &Arc<ServerContext>) {
    resources
        .common
        .repos
        .usage_counters
        .delete_old_counters("9999")
        .await
        .unwrap();
}

/// Another instance of the backend over the same database: it shares the
/// database and nothing held in memory, the rate limiter included. Built
/// after the first one, it also stands for that one restarted.
fn another_instance(resources: &Arc<ServerContext>, per_window: u32) -> Arc<ServerContext> {
    with_strava_window(resources, per_window)
}

/// Run passes, each in a fresh window, until no
/// athlete is owed a walk. Panics when that takes more than 30 passes.
async fn walk_to_completion(resources: &Arc<ServerContext>) {
    for _ in 0..30 {
        open_window(resources).await;
        run_personal_best_seed_pass(resources).await.unwrap();
        let owed = resources
            .common
            .repos
            .personal_bests
            .list_personal_best_seed_candidates("strava", 10)
            .await
            .unwrap();
        if owed.is_empty() {
            return;
        }
    }
    panic!("the walk did not complete in 30 passes");
}

async fn walk_complete(resources: &ServerContext, athlete: &Athlete) -> bool {
    resources
        .personal_bests()
        .is_seed_complete(athlete.user_id, athlete.tenant_id, "strava")
        .await
        .unwrap()
}

async fn stored_5k(resources: &ServerContext, athlete: &Athlete) -> Option<f64> {
    resources
        .common
        .repos
        .personal_bests
        .personal_bests(athlete.user_id, athlete.tenant_id)
        .await
        .unwrap()
        .into_iter()
        .find(|best| best.distance == "5k")
        .map(|best| best.elapsed_seconds)
}

/// The `personal_record` notices the athlete holds, as `(distance, time)`.
async fn record_notices(resources: &ServerContext, athlete: &Athlete) -> Vec<(Value, Value)> {
    sleep(Duration::from_millis(400)).await;
    let service = resources
        .common
        .notification_service
        .as_ref()
        .expect("the server boots a notification service");
    let (rows, _, _) = service
        .list_notifications(
            athlete.user_id,
            CommereTenantId(athlete.tenant_id.as_uuid()),
            50,
            0,
            None,
            false,
        )
        .await
        .unwrap();
    rows.into_iter()
        .filter(|row| row.notification_type == "personal_record")
        .map(|row| {
            let params = event_params(row.data.as_ref())
                .expect("a record stores its parameters")
                .clone();
            (params["distance"].clone(), params["time_display"].clone())
        })
        .collect()
}

/// The activity a sync lists for `mocked`, as the Strava mapper builds it.
fn synced(mocked: MockActivity) -> Activity {
    ActivityBuilder::new(
        mocked.id.to_string(),
        "Morning activity".to_owned(),
        SportType::Run,
        Utc.timestamp_opt(mocked.start, 0).unwrap(),
        u64::from(mocked.seconds_per_km * mocked.km),
        "strava".to_owned(),
    )
    .distance_meters(f64::from(mocked.km) * 1000.0)
    .build()
}

/// A sync scanning `activities` for the athlete, as the webhook does.
async fn sync_scan(resources: &Arc<ServerContext>, athlete: &Athlete, activities: &[Activity]) {
    let provider = strava_provider(resources, athlete.user_id, &athlete.tenant_id.to_string())
        .await
        .expect("the athlete's Strava provider");
    resources
        .personal_bests()
        .scan_new_runs(
            provider.as_ref(),
            athlete.user_id,
            athlete.tenant_id,
            activities,
        )
        .await
        .unwrap();
}

/// A walk the budget stops part-way is picked up by a restarted process
/// from the cursor it saved: every past run's streams are requested exactly
/// once across both, rides never, the stored 5 km is the fastest of the whole
/// history, and nothing is told.
#[tokio::test]
#[serial]
async fn the_walk_measures_every_past_run_exactly_once_across_a_restart() {
    let (api_base, mock) = mock_strava().await;
    let (resources, _env) = context_pointed_at(&api_base, 16).await;
    let runs = [
        run(101, 400, 330),
        run(102, 300, 300),
        run(103, 200, 280),
        run(104, 100, 310),
        run(105, 10, 320),
    ];
    let mut history = runs.to_vec();
    history.extend([ride(201, 350), ride(202, 50)]);
    let athlete = linked_athlete(&resources, &mock, "seed_restart@example.com", 11, history).await;

    // Sixteen requests a window, four of them the walk's: one listing, then
    // runs 105 and 104 from the first page, then the second listing.
    open_window(&resources).await;
    let report = run_personal_best_seed_pass(&resources).await.unwrap();
    assert_eq!(
        report,
        SeedPassReport {
            completed: 0,
            paused: 1,
            held: 0,
            budget_spent: true
        }
    );
    assert_eq!(mock.streams_hits(105), 1);
    assert_eq!(mock.streams_hits(104), 1);
    assert_eq!(mock.total_streams_hits(), 2, "the budget stopped the walk");
    assert!(!walk_complete(&resources, &athlete).await);

    let resumed = another_instance(&resources, 16);
    walk_to_completion(&resumed).await;

    for measured in runs {
        assert_eq!(
            mock.streams_hits(measured.id),
            1,
            "run {} is measured exactly once",
            measured.id
        );
    }
    assert_eq!(mock.streams_hits(201), 0, "a ride is never measured");
    assert_eq!(mock.streams_hits(202), 0, "a ride is never measured");
    assert_eq!(mock.total_streams_hits(), runs.len());
    assert!(walk_complete(&resumed, &athlete).await);
    assert_eq!(
        stored_5k(&resumed, &athlete).await,
        Some(1400.0),
        "run 103's 5 km at 4:40/km is the all-time best"
    );
    assert!(
        record_notices(&resumed, &athlete).await.is_empty(),
        "the walk stores the history's bests and tells nothing"
    );
}

/// When the walk's share of the Strava window is spent the pass stops; a
/// second pass in the same window makes no request at all and leaves the
/// other three quarters of the window to live requests, and the next window
/// resumes the walk where it stopped until it completes.
#[tokio::test]
#[serial]
async fn the_walk_stops_when_the_budget_says_stop_and_resumes_in_the_next_window() {
    let (api_base, mock) = mock_strava().await;
    let (resources, _env) = context_pointed_at(&api_base, 12).await;
    let runs = [
        run(301, 90, 300),
        run(302, 60, 300),
        run(303, 30, 300),
        run(304, 5, 300),
    ];
    let athlete = linked_athlete(
        &resources,
        &mock,
        "seed_budget@example.com",
        12,
        runs.to_vec(),
    )
    .await;

    // Twelve requests a window, three of them the walk's.
    open_window(&resources).await;
    let first = run_personal_best_seed_pass(&resources).await.unwrap();
    assert!(first.budget_spent);
    assert_eq!(mock.list_hits(&athlete.token), 1);
    assert_eq!(mock.total_streams_hits(), 2);

    let second = run_personal_best_seed_pass(&resources).await.unwrap();
    assert!(second.budget_spent, "the window's share is still spent");
    assert_eq!(
        mock.list_hits(&athlete.token),
        1,
        "no listing in a spent window"
    );
    assert_eq!(
        mock.total_streams_hits(),
        2,
        "no streams request in a spent window"
    );
    // The walk's requests are signed by the server's app; a live request
    // under it counts in the same windows, in the same database.
    let live = ProviderRateLimiter::new(
        Arc::clone(&resources.common.repos.usage_counters),
        Arc::clone(&resources.common.repos.provider_connections),
    );
    live.set_budgets("strava", &[(12, ONE_DAY)]);
    assert_eq!(
        live.acquire("strava", "test_client", None).await.unwrap(),
        RateLimitStatus::Allowed,
        "three quarters of the window stay open to live requests"
    );
    assert!(!walk_complete(&resources, &athlete).await);

    walk_to_completion(&resources).await;
    for measured in runs {
        assert_eq!(mock.streams_hits(measured.id), 1, "run {}", measured.id);
    }
    assert!(walk_complete(&resources, &athlete).await);
}

/// While the walk is incomplete a synced run faster than every best known so
/// far — even with an earlier sync's run already stored — is stored and not
/// told. The walk then skips the runs the syncs measured, and completing it
/// tells nothing either.
#[tokio::test]
#[serial]
async fn no_record_is_told_while_the_walk_is_incomplete() {
    let (api_base, mock) = mock_strava().await;
    let (resources, _env) = context_pointed_at(&api_base, 100).await;
    let old = run(401, 300, 300);
    let earlier = run(402, 20, 290);
    let athlete = linked_athlete(
        &resources,
        &mock,
        "seed_silent@example.com",
        13,
        vec![old, earlier],
    )
    .await;

    sync_scan(&resources, &athlete, &[synced(earlier)]).await;
    assert_eq!(stored_5k(&resources, &athlete).await, Some(1450.0));

    let fast = run(403, 0, 270);
    mock.upload(&athlete.token, fast);
    sync_scan(&resources, &athlete, &[synced(fast)]).await;
    assert_eq!(
        stored_5k(&resources, &athlete).await,
        Some(1350.0),
        "the faster 5 km is stored"
    );
    assert!(
        record_notices(&resources, &athlete).await.is_empty(),
        "no record is told against a partial history"
    );

    walk_to_completion(&resources).await;
    assert_eq!(mock.streams_hits(401), 1);
    assert_eq!(mock.streams_hits(402), 1, "the walk skips a synced run");
    assert_eq!(mock.streams_hits(403), 1, "the walk skips a synced run");
    assert!(walk_complete(&resources, &athlete).await);
    assert_eq!(stored_5k(&resources, &athlete).await, Some(1350.0));
    assert!(record_notices(&resources, &athlete).await.is_empty());
}

/// Once the walk is complete, a synced run faster than the last weeks but
/// slower than the all-time best tells nothing, and one beating the all-time
/// best sends exactly one notice with its time.
#[tokio::test]
#[serial]
async fn after_the_walk_only_a_run_beating_the_all_time_best_is_told_once() {
    let (api_base, mock) = mock_strava().await;
    let (resources, _env) = context_pointed_at(&api_base, 100).await;
    let athlete = linked_athlete(
        &resources,
        &mock,
        "seed_told@example.com",
        14,
        vec![run(501, 500, 280), run(502, 30, 300)],
    )
    .await;
    walk_to_completion(&resources).await;
    assert!(walk_complete(&resources, &athlete).await);
    assert_eq!(stored_5k(&resources, &athlete).await, Some(1400.0));
    assert!(record_notices(&resources, &athlete).await.is_empty());

    let recent_best = run(503, 1, 290);
    mock.upload(&athlete.token, recent_best);
    sync_scan(&resources, &athlete, &[synced(recent_best)]).await;
    assert!(
        record_notices(&resources, &athlete).await.is_empty(),
        "faster than the last weeks is not an all-time record"
    );
    assert_eq!(stored_5k(&resources, &athlete).await, Some(1400.0));

    let record = run(504, 0, 270);
    mock.upload(&athlete.token, record);
    sync_scan(&resources, &athlete, &[synced(record)]).await;
    assert_eq!(
        record_notices(&resources, &athlete).await,
        vec![(json!("5k"), json!("22:30"))],
        "one notice: the 5 km in 22:30"
    );
    assert_eq!(stored_5k(&resources, &athlete).await, Some(1350.0));
}

/// Each athlete's walk requests the streams of each of their runs once, so
/// their streams requests equal their runs, plus one listing per page and the
/// empty one that ends the walk.
#[tokio::test]
#[serial]
async fn each_athletes_streams_requests_equal_their_runs() {
    let (api_base, mock) = mock_strava().await;
    let (resources, _env) = context_pointed_at(&api_base, 100).await;
    let first_runs = [
        run(601, 40, 300),
        run(602, 30, 300),
        run(603, 20, 300),
        run(604, 10, 300),
    ];
    let mut first_history = first_runs.to_vec();
    first_history.push(ride(651, 15));
    let first = linked_athlete(&resources, &mock, "seed_a@example.com", 15, first_history).await;
    let second_runs = [run(701, 40, 320), run(702, 10, 320)];
    let mut second_history = second_runs.to_vec();
    second_history.extend([ride(751, 30), ride(752, 20)]);
    let second = linked_athlete(&resources, &mock, "seed_b@example.com", 16, second_history).await;

    open_window(&resources).await;
    let report = run_personal_best_seed_pass(&resources).await.unwrap();
    assert_eq!(
        report,
        SeedPassReport {
            completed: 2,
            paused: 0,
            held: 0,
            budget_spent: false
        }
    );

    let first_streams: usize = first_runs.iter().map(|r| mock.streams_hits(r.id)).sum();
    let second_streams: usize = second_runs.iter().map(|r| mock.streams_hits(r.id)).sum();
    assert_eq!(first_streams, first_runs.len());
    assert_eq!(second_streams, second_runs.len());
    assert_eq!(
        mock.total_streams_hits(),
        first_runs.len() + second_runs.len(),
        "no ride is measured"
    );
    // Five activities: pages of three and two, then the empty page; four:
    // three and one, then the empty page.
    assert_eq!(mock.list_hits(&first.token), 3);
    assert_eq!(mock.list_hits(&second.token), 3);
    assert!(walk_complete(&resources, &first).await);
    assert!(walk_complete(&resources, &second).await);
}

/// A run whose streams request fails while the provider answers the next
/// request — the next run's streams, or the listing past the oldest
/// activity — failed on its own: it is recorded unmeasured and asked for
/// once, and the walk goes on to complete.
#[tokio::test]
#[serial]
async fn a_run_failing_alone_is_recorded_unmeasured_and_the_walk_completes() {
    let (api_base, mock) = mock_strava().await;
    let (resources, _env) = context_pointed_at(&api_base, 100).await;
    let runs = [
        run(801, 50, 330),
        run(802, 40, 300),
        run(803, 30, 250),
        run(804, 20, 300),
        run(805, 10, 300),
    ];
    // 803 fails between two runs that answer; 801, the oldest, fails with
    // only the final listing after it.
    mock.failing.lock().unwrap().extend([803, 801]);
    let athlete = linked_athlete(
        &resources,
        &mock,
        "seed_alone@example.com",
        17,
        runs.to_vec(),
    )
    .await;

    walk_to_completion(&resources).await;

    for asked in runs {
        assert_eq!(mock.streams_hits(asked.id), 1, "run {}", asked.id);
    }
    assert!(walk_complete(&resources, &athlete).await);
    let repo = &resources.common.repos.personal_bests;
    for unmeasured in ["803", "801"] {
        assert!(
            repo.is_activity_scanned(athlete.user_id, athlete.tenant_id, "strava", unmeasured)
                .await
                .unwrap(),
            "run {unmeasured} is recorded, so it is never asked for again"
        );
    }
    assert_eq!(
        stored_5k(&resources, &athlete).await,
        Some(1500.0),
        "the failed 803 set nothing"
    );
}

/// Two runs failing in a row is Strava refusing, not the runs: the walk
/// pauses with neither recorded, and once Strava answers again the next pass
/// asks for both again and completes.
#[tokio::test]
#[serial]
async fn two_runs_failing_in_a_row_pause_the_walk_until_strava_answers() {
    let (api_base, mock) = mock_strava().await;
    let (resources, _env) = context_pointed_at(&api_base, 100).await;
    let runs = [run(901, 30, 300), run(902, 20, 290), run(903, 10, 300)];
    mock.failing.lock().unwrap().extend([903, 902]);
    let athlete = linked_athlete(
        &resources,
        &mock,
        "seed_refused@example.com",
        18,
        runs.to_vec(),
    )
    .await;

    open_window(&resources).await;
    let refused = run_personal_best_seed_pass(&resources).await.unwrap();
    assert_eq!(
        refused,
        SeedPassReport {
            completed: 0,
            paused: 1,
            held: 0,
            budget_spent: false
        }
    );
    let repo = &resources.common.repos.personal_bests;
    for held in ["903", "902"] {
        assert!(!repo
            .is_activity_scanned(athlete.user_id, athlete.tenant_id, "strava", held)
            .await
            .unwrap());
    }
    assert_eq!(mock.streams_hits(901), 0, "the walk stopped at the refusal");

    mock.failing.lock().unwrap().clear();
    walk_to_completion(&resources).await;
    assert_eq!(mock.streams_hits(903), 2, "asked again once Strava answers");
    assert_eq!(mock.streams_hits(902), 2, "asked again once Strava answers");
    assert_eq!(mock.streams_hits(901), 1);
    assert!(walk_complete(&resources, &athlete).await);
    assert_eq!(stored_5k(&resources, &athlete).await, Some(1450.0));
}

/// Two instances walking at once, each its own athlete, draw on the one
/// budget counted in the database: together they make exactly the walk's
/// share of the window, not that share each.
#[tokio::test]
#[serial]
async fn two_instances_walking_at_once_never_exceed_the_walks_share_together() {
    let (api_base, mock) = mock_strava().await;
    let (first, _env) = context_pointed_at(&api_base, 16).await;
    let second = another_instance(&first, 16);
    let history = |base: u64| {
        (0..6_u32)
            .map(|i| run(base + u64::from(i), 60 - 5 * i64::from(i), 300))
            .collect()
    };
    linked_athlete(&first, &mock, "seed_one@example.com", 21, history(1_001)).await;
    linked_athlete(&first, &mock, "seed_two@example.com", 22, history(2_001)).await;

    // Sixteen requests a window, four of them the walk's, for both.
    open_window(&first).await;
    let (a, b) = tokio::join!(
        run_personal_best_seed_pass(&first),
        run_personal_best_seed_pass(&second)
    );
    let (a, b) = (a.unwrap(), b.unwrap());

    assert_eq!(
        mock.total_requests(),
        4,
        "one share between the two instances, not one each"
    );
    assert!(a.budget_spent || b.budget_spent);
    assert_eq!(a.completed + b.completed, 0);
}

/// Two instances passing at once never walk the same athlete together: one
/// holds the athlete's lease and walks, the other passes them over, and no
/// run's streams are asked for twice or by both at once.
#[tokio::test]
#[serial]
async fn two_seed_workers_never_walk_the_same_athlete_at_once() {
    let (api_base, mock) = mock_strava().await;
    let (first, _env) = context_pointed_at(&api_base, 100).await;
    let second = another_instance(&first, 100);
    let runs: Vec<MockActivity> = (0..5_u32)
        .map(|i| run(3_001 + u64::from(i), 50 - 5 * i64::from(i), 300))
        .collect();
    let athlete = linked_athlete(&first, &mock, "seed_shared@example.com", 23, runs.clone()).await;
    // Slow streams, so the two passes overlap for as long as the walk lasts.
    *mock.streams_delay.lock().unwrap() = Duration::from_millis(60);

    open_window(&first).await;
    let (a, b) = tokio::join!(
        run_personal_best_seed_pass(&first),
        run_personal_best_seed_pass(&second)
    );
    let (a, b) = (a.unwrap(), b.unwrap());

    assert_eq!(
        a.completed + b.completed,
        1,
        "one instance walked the athlete"
    );
    assert_eq!(a.held + b.held, 1, "the other found them held");
    assert_eq!(mock.max_in_flight(&athlete.token), 1);
    for walked in runs {
        assert_eq!(mock.streams_hits(walked.id), 1, "run {}", walked.id);
    }
    assert_eq!(
        mock.list_hits(&athlete.token),
        3,
        "two pages and the empty one"
    );
}

/// While another instance holds the athlete's lease a pass leaves them alone;
/// once that lease has run out — its instance died holding it — the next
/// pass takes the athlete over and walks them.
#[tokio::test]
#[serial]
async fn an_expired_lease_is_taken_over() {
    let (api_base, mock) = mock_strava().await;
    let (resources, _env) = context_pointed_at(&api_base, 100).await;
    let runs = [
        run(4_001, 30, 300),
        run(4_002, 20, 300),
        run(4_003, 10, 300),
    ];
    let athlete = linked_athlete(
        &resources,
        &mock,
        "seed_lease@example.com",
        24,
        runs.to_vec(),
    )
    .await;
    let ledger = &resources.common.repos.worker_runs;
    let lease = athlete_lease_name(athlete.user_id, athlete.tenant_id);
    let now_ms = Utc::now().timestamp_millis();
    assert!(ledger
        .claim_worker_run(&lease, 0, now_ms, 15 * 60 * 1_000)
        .await
        .unwrap());

    open_window(&resources).await;
    let held = run_personal_best_seed_pass(&resources).await.unwrap();
    assert_eq!(held.held, 1);
    assert_eq!(held.completed, 0);
    assert_eq!(mock.total_requests(), 0, "a held athlete costs no request");

    // The holder died: its lease ran out a moment ago.
    ledger.defer_worker_run(&lease, now_ms - 1).await.unwrap();
    let taken = run_personal_best_seed_pass(&resources).await.unwrap();
    assert_eq!(taken.completed, 1);
    assert_eq!(taken.held, 0);
    for walked in runs {
        assert_eq!(mock.streams_hits(walked.id), 1, "run {}", walked.id);
    }
    assert!(walk_complete(&resources, &athlete).await);
}

/// A sync's scan and the walk draw on one budget: the requests the scan made
/// leave the walk only what remains of its share, and both are in the one
/// count in the database.
#[tokio::test]
#[serial]
async fn the_webhook_scan_and_the_walk_share_the_budget() {
    let (api_base, mock) = mock_strava().await;
    let (resources, _env) = context_pointed_at(&api_base, 16).await;
    let history = [
        run(5_001, 90, 300),
        run(5_002, 60, 300),
        run(5_003, 30, 300),
    ];
    let athlete = linked_athlete(
        &resources,
        &mock,
        "seed_shared_budget@example.com",
        25,
        history.to_vec(),
    )
    .await;

    // Sixteen requests a window, four of them the walk's.
    open_window(&resources).await;
    let uploads = [run(5_101, 1, 290), run(5_102, 0, 280)];
    for upload in uploads {
        mock.upload(&athlete.token, upload);
    }
    sync_scan(&resources, &athlete, &uploads.map(synced)).await;
    assert_eq!(
        mock.total_streams_hits(),
        2,
        "the scan measured the uploads"
    );

    let report = run_personal_best_seed_pass(&resources).await.unwrap();
    assert!(report.budget_spent);
    let walked: usize = history.iter().map(|r| mock.streams_hits(r.id)).sum();
    assert_eq!(
        walked, 1,
        "the scan's two requests left the walk one listing and one run"
    );
    let counted = resources
        .common
        .repos
        .usage_counters
        .get_counter(
            PLATFORM_SCOPE,
            PLATFORM_SCOPE,
            &budget_counter_key("strava", "test_client"),
            &budget_period(Utc::now(), ONE_DAY),
        )
        .await
        .unwrap();
    assert_eq!(counted.value, 4, "two scan requests and two walk requests");
}
