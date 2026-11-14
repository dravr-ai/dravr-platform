// ABOUTME: The detail walk: a window's stored rides read through the Strava grant for their segment efforts, newest first
// ABOUTME: Stored under the grant's key with its copy of a session-only ride, an unreadable ride walked past, a spent day paused

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Detail walks (carnet#1483).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// An athlete's stored rides carry no segment efforts until a detail read of
// each, which only the Strava API answers with them. A detail walk reads the
// window's rides through the athlete's grant, ten a step, in its own day
// budget, and stores each answer under the grant's key: a ride only the
// session listed gets the grant's copy first, which the grant's purge takes
// back. A ride the API cannot read is walked past and kept. A spent day
// pauses the walk until the day ends. An ask while one is owed widens it.
//
// Strava is a loopback stand-in (a test double, per the repo's mock rule)
// that answers each detail read with one effort and counts the reads.
#![cfg(all(
    feature = "tools-data",
    feature = "provider-strava",
    feature = "provider-garmin",
    feature = "health-sync"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::history_walk_api_test::{open_window, server, strava_athlete};
use crate::history_walk_test::{age_job_rows, paused_retries_due};

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{TimeZone, Utc};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::coverage::Pending;
use pierre_core::models::{Activity, ActivityBuilder, SportType, TenantId};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::{
    BackfillJobKind, BackfillJobState, CachedActivityRow, CachedRowsQuery, DetailWalkRow,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::activity_backfill::detail_walk::{
    ask_detail_walk, detail_walk_pending, segment_detail_backend, DetailWalkAsk, DetailWalkAsked,
};
use pierre_tool_runtime::activity_backfill_resume::resume_backfill_jobs;
use pierre_tool_runtime::runtime::{
    BackfillCompletion, BackfillNotifier, DetailWalkCompletion, HistoryWalkCompletion, ReauthRetry,
    ToolRuntime,
};
use serde_json::{json, Value};
use sqlx::Row;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

/// Seconds in a day.
pub(crate) const DAY: i64 = 86_400;

/// How long a walk gets to end before the test gives up.
const WALK_DEADLINE: Duration = Duration::from_secs(30);

/// The key the athlete's session files its rides under.
pub(crate) const SESSION: &str = "sciotte";

/// What the Strava stand-in answers and saw.
#[derive(Default)]
pub(crate) struct MockStrava {
    /// Every detail read's id, in the order they came.
    details: Mutex<Vec<i64>>,
    /// The ids it answers `404` for.
    pub(crate) missing: Mutex<HashSet<i64>>,
    /// The ids it answers `503` for: Strava down, not the ride gone.
    pub(crate) failing: Mutex<HashSet<i64>>,
    /// The gate the next detail read waits on once it is counted.
    hold: Mutex<Option<Arc<Notify>>>,
    /// Raised when a detail read reached the gate.
    held: Notify,
}

impl MockStrava {
    pub(crate) fn detail_reads(&self) -> Vec<i64> {
        self.details.lock().unwrap().clone()
    }

    /// Hold the next detail read, once counted, until the returned gate is
    /// notified.
    pub(crate) fn hold_next_read(&self) -> Arc<Notify> {
        let gate = Arc::new(Notify::new());
        *self.hold.lock().unwrap() = Some(Arc::clone(&gate));
        gate
    }

    pub(crate) async fn read_held(&self) {
        timeout(Duration::from_secs(10), self.held.notified())
            .await
            .expect("no detail read reached the hold");
    }
}

/// `GET /activities/{id}?include_all_efforts=true`: the ride whose id is its
/// start, with one effort on Camillien-Houde.
async fn activity_detail(
    State(mock): State<Arc<MockStrava>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, StatusCode> {
    mock.details.lock().unwrap().push(id);
    let hold = mock.hold.lock().unwrap().take();
    if let Some(gate) = hold {
        mock.held.notify_one();
        gate.notified().await;
    }
    if mock.missing.lock().unwrap().contains(&id) {
        return Err(StatusCode::NOT_FOUND);
    }
    if mock.failing.lock().unwrap().contains(&id) {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let start = Utc.timestamp_opt(id, 0).unwrap();
    Ok(Json(json!({
        "id": id,
        "name": "Morning ride",
        "type": "Ride",
        "sport_type": "Ride",
        "start_date": start.to_rfc3339(),
        "distance": 20_000.0,
        "elapsed_time": 3_600,
        "total_elevation_gain": 10.0,
        "segment_efforts": [{
            "id": id + 1,
            "name": "Camillien-Houde",
            "elapsed_time": 412,
            "moving_time": 410,
            "distance": 1_820.5,
            "start_date": (start + chrono::Duration::minutes(10)).to_rfc3339(),
            "pr_rank": 2,
            "segment": { "id": 611_413, "name": "Camillien-Houde", "distance": 1_820.5 }
        }]
    })))
}

pub(crate) async fn mock_strava() -> (String, Arc<MockStrava>) {
    let mock = Arc::new(MockStrava::default());
    let app = Router::new()
        .route("/activities/{id}", get(activity_detail))
        .with_state(Arc::clone(&mock));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), mock)
}

/// A ride started at `start` as `key` lists it, its id its start: the
/// session's capture names Strava as its source.
pub(crate) fn ride(key: &str, start: i64) -> Activity {
    ride_listed_at(key, start, start)
}

/// The ride whose Strava id is `id` (its real start), as `key` lists it
/// started at `start`: a session's list read can carry only the day.
pub(crate) fn ride_listed_at(key: &str, id: i64, start: i64) -> Activity {
    let built = ActivityBuilder::new(
        id.to_string(),
        "Morning ride",
        SportType::Ride,
        Utc.timestamp_opt(start, 0).unwrap(),
        3_600,
        key,
    );
    if key == SESSION {
        built.source(oauth_providers::STRAVA).build()
    } else {
        built.build()
    }
}

/// Store `starts` as the rides `key` listed for the athlete.
pub(crate) async fn store_rides(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    key: &str,
    starts: &[i64],
) {
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let rides: Vec<Activity> = starts.iter().map(|start| ride(key, *start)).collect();
    runtime
        .live_activities()
        .ingest(user_id, tenant, key, &rides, Utc::now())
        .await
        .unwrap();
}

/// The athlete's stored rows under `key`, newest first.
pub(crate) async fn rows_under(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    key: &str,
) -> Vec<CachedActivityRow> {
    let keys = [key];
    resources
        .common
        .repos
        .activity_cache
        .get_cached_activity_rows(
            user_id,
            &tenant,
            &CachedRowsQuery::window(Utc.timestamp_opt(0, 0).unwrap(), Utc::now(), 500)
                .under_keys(&keys),
        )
        .await
        .unwrap()
}

/// The athlete's detail walk row through the grant.
pub(crate) async fn walk_row(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> Option<DetailWalkRow> {
    resources
        .common
        .repos
        .activity_backfill_jobs
        .detail_walk_row(user_id, tenant, oauth_providers::STRAVA)
        .await
        .unwrap()
}

/// How far back a walk's ask is moved while the sweep runs it: past the
/// sweep's two-minute grace, so the sweep takes the row.
const AGED_MS: i64 = 10 * 60 * 1_000;

/// Run the walk the sweep's way, one step a pass, in a fresh window each
/// pass, until its row ends.
///
/// The sweep takes only a row asked longer ago than its grace, so the row's
/// ask is moved back [`AGED_MS`] while it runs: each walk keeps an ask time
/// of its own, which keys its completion post. Once the row ends it gets its
/// own ask time back, which a later ask reads off it: what was stored before
/// it was asked is what it tried.
pub(crate) async fn walk_ended(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> DetailWalkRow {
    let deadline = Instant::now() + WALK_DEADLINE;
    let asked_ms = walk_row(resources, user_id, tenant)
        .await
        .map(|row| row.created_at_ms);
    loop {
        let row = walk_row(resources, user_id, tenant).await;
        if let Some(row) = row
            .as_ref()
            .filter(|row| row.state != BackfillJobState::Owed)
        {
            let Some(asked_ms) = asked_ms else {
                return row.clone();
            };
            alter_walk_row(resources, user_id, &format!("created_at_ms = {asked_ms}")).await;
            return walk_row(resources, user_id, tenant)
                .await
                .expect("the ended walk's row");
        }
        assert!(
            Instant::now() < deadline,
            "the walk did not end within {WALK_DEADLINE:?} (row: {row:?})"
        );
        open_window(resources).await;
        paused_retries_due(resources, user_id).await;
        match asked_ms {
            Some(asked_ms) => {
                let aged = asked_ms - AGED_MS;
                alter_walk_row(resources, user_id, &format!("created_at_ms = {aged}")).await;
            }
            None => age_job_rows(resources, user_id).await,
        }
        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        resume_backfill_jobs(runtime).await.unwrap();
        sleep(Duration::from_millis(50)).await;
    }
}

/// The window `[after, now)` of every sport.
pub(crate) fn since(after: i64) -> DetailWalkAsk<'static> {
    DetailWalkAsk {
        after_ts: after,
        before_ts: Utc::now().timestamp(),
        sport: None,
    }
}

/// The grant reads a ride the session alone listed: its answer lands under
/// the grant's key, as the grant's own copy of the ride, never on the
/// session's row. A ride the grant holds already is detailed in place. The
/// walk reads each once and ends walked.
#[tokio::test]
async fn a_walk_stores_efforts_under_the_grant_and_copies_a_session_only_ride() {
    let (api_base, mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    let (session_only, both) = (now - 10 * DAY, now - 20 * DAY);
    store_rides(&resources, user_id, tenant, SESSION, &[session_only, both]).await;
    store_rides(
        &resources,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        &[both],
    )
    .await;
    open_window(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert_eq!(
        segment_detail_backend(&runtime, user_id, tenant, oauth_providers::STRAVA).await,
        Some(oauth_providers::STRAVA.to_owned()),
        "bug: the live grant, whose detail reads carry efforts, is the walk's backend"
    );
    let asked = ask_detail_walk(
        &runtime,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        since(now - 30 * DAY),
    )
    .await;
    assert_eq!(
        asked,
        DetailWalkAsked::Started { units_total: 2 },
        "bug: the two rides, the session's and the grant's copies of one counted once, are owed"
    );
    let ended = walk_ended(&resources, user_id, tenant).await;

    assert_eq!(
        ended.state,
        BackfillJobState::Walked,
        "bug: the walk ends at its floor"
    );
    assert_eq!(
        (ended.progress.units_done, ended.progress.rows_landed),
        (2, 2),
        "bug: both rides are read and stored"
    );
    let mut reads = mock.detail_reads();
    reads.sort_unstable();
    assert_eq!(
        reads,
        vec![both, session_only],
        "bug: each ride is read once, through the grant"
    );
    let grant_rows = rows_under(&resources, user_id, tenant, oauth_providers::STRAVA).await;
    let grant_ids: Vec<&str> = grant_rows.iter().map(|row| row.activity.id()).collect();
    assert_eq!(
        grant_ids,
        vec![session_only.to_string(), both.to_string()],
        "bug: the session-only ride gets the grant's copy, beside the ride the grant held"
    );
    assert!(
        grant_rows.iter().all(|row| row.segment_read),
        "bug: every grant row carries the efforts the grant read: {grant_rows:?}"
    );
    assert!(
        rows_under(&resources, user_id, tenant, SESSION)
            .await
            .iter()
            .all(|row| !row.segment_read),
        "bug: Strava API efforts landed on the session's rows"
    );
}

/// A ride the API answers `404` for is walked past: counted read, never
/// stored, and never deleted from the store. The walk still ends walked.
#[tokio::test]
async fn an_unreadable_ride_is_walked_past_and_kept() {
    let (api_base, mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    let starts = [now - 5 * DAY, now - 6 * DAY, now - 7 * DAY];
    mock.missing.lock().unwrap().insert(starts[1]);
    store_rides(&resources, user_id, tenant, SESSION, &starts).await;
    open_window(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let asked = ask_detail_walk(
        &runtime,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        since(now - 10 * DAY),
    )
    .await;
    assert_eq!(
        asked,
        DetailWalkAsked::Started { units_total: 3 },
        "the three rides are owed"
    );
    let ended = walk_ended(&resources, user_id, tenant).await;

    assert_eq!(
        ended.state,
        BackfillJobState::Walked,
        "bug: a ride the API cannot read keeps the walk from its floor"
    );
    assert_eq!(
        (ended.progress.units_done, ended.progress.rows_landed),
        (3, 2),
        "bug: the unreadable ride is counted read and not stored"
    );
    assert_eq!(
        mock.detail_reads().len(),
        3,
        "bug: the unreadable ride is read again instead of walked past"
    );
    let session_ids: Vec<String> = rows_under(&resources, user_id, tenant, SESSION)
        .await
        .iter()
        .map(|row| row.activity.id().to_owned())
        .collect();
    assert!(
        session_ids.contains(&starts[1].to_string()),
        "bug: a ride the API answered 404 for was deleted from the store: {session_ids:?}"
    );
}

/// Strava failing (`503`) is not the ride gone: the walk stops at it, keeps
/// what it read before it, counts it neither read nor walked past, and reads
/// it again once Strava answers. Only then does the walk end walked.
#[tokio::test]
async fn a_ride_strava_fails_on_is_read_again_not_walked_past() {
    let (api_base, mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    let starts = [now - 5 * DAY, now - 6 * DAY, now - 7 * DAY];
    mock.failing.lock().unwrap().insert(starts[1]);
    store_rides(&resources, user_id, tenant, SESSION, &starts).await;
    open_window(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let asked = ask_detail_walk(
        &runtime,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        since(now - 10 * DAY),
    )
    .await;
    assert_eq!(
        asked,
        DetailWalkAsked::Started { units_total: 3 },
        "the three rides are owed"
    );
    let deadline = Instant::now() + WALK_DEADLINE;
    let settled = loop {
        let row = walk_row(&resources, user_id, tenant)
            .await
            .expect("the walk's row");
        if row.state != BackfillJobState::Owed || row.progress.units_done > 0 {
            break row;
        }
        assert!(
            Instant::now() < deadline,
            "the walk's first step did not settle within {WALK_DEADLINE:?} (row: {row:?})"
        );
        sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(
        settled.state,
        BackfillJobState::Owed,
        "bug: a walk ended over a ride Strava failed on: {settled:?}"
    );
    assert_eq!(
        (settled.progress.units_done, settled.progress.rows_landed),
        (1, 1),
        "bug: the ride Strava failed on is counted done: {settled:?}"
    );
    assert!(
        !mock.detail_reads().contains(&starts[2]),
        "bug: the walk read on past a ride Strava failed on: {:?}",
        mock.detail_reads()
    );

    mock.failing.lock().unwrap().clear();
    let ended = walk_ended(&resources, user_id, tenant).await;
    assert_eq!(
        ended.state,
        BackfillJobState::Walked,
        "the walk ends walked once Strava answers: {ended:?}"
    );
    assert_eq!(
        (ended.progress.units_done, ended.progress.rows_landed),
        (3, 3),
        "bug: the ride Strava failed on was not read again: {ended:?}"
    );
    assert_eq!(
        mock.detail_reads()
            .iter()
            .filter(|id| **id == starts[1])
            .count(),
        2,
        "bug: the ride Strava failed on was not read again once it answered: {:?}",
        mock.detail_reads()
    );
}

/// Strava failing on a ride for every attempt the walk has ends it
/// incomplete, never walked: the athlete is told nothing, and the next ask
/// walks the window afresh, never reporting the ride unreadable.
#[tokio::test]
async fn a_walk_strava_keeps_failing_ends_incomplete_untold() {
    let (api_base, mock) = mock_strava().await;
    let told = Arc::new(Told::default());
    let resources = telling(&server(&api_base, 400).await, &told);
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    let starts = [now - 5 * DAY, now - 6 * DAY];
    mock.failing.lock().unwrap().insert(starts[1]);
    store_rides(&resources, user_id, tenant, SESSION, &starts).await;
    open_window(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let asked = ask_detail_walk(
        &runtime,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        since(now - 10 * DAY),
    )
    .await;
    assert_eq!(
        asked,
        DetailWalkAsked::Started { units_total: 2 },
        "the two rides are owed"
    );
    let ended = walk_ended(&resources, user_id, tenant).await;

    assert_eq!(
        ended.state,
        BackfillJobState::Incomplete,
        "bug: a walk Strava kept failing ended as if it walked its window: {ended:?}"
    );
    assert!(
        told.walks.lock().unwrap().is_empty(),
        "bug: the athlete was told a walk Strava kept failing is done: {:?}",
        told.walks.lock().unwrap()
    );
    let asked = ask_detail_walk(
        &runtime,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        since(now - 10 * DAY),
    )
    .await;
    assert_eq!(
        asked,
        DetailWalkAsked::Started { units_total: 1 },
        "bug: the ride Strava failed on reads as unreadable, not walked again"
    );
}

/// The walk's next retry once a spent budget released its row.
async fn next_attempt_ms(resources: &Arc<ServerContext>, user_id: Uuid) -> Option<i64> {
    let sql = "SELECT next_attempt_ms FROM activity_backfill_jobs \
               WHERE user_id = $1 AND kind = 'detail_walk' AND leased_until_ms = 0 \
                 AND outcome IS NULL";
    macro_rules! read {
        ($pool:expr) => {{
            sqlx::query(sql)
                .bind(user_id.to_string())
                .fetch_optional($pool)
                .await
                .unwrap()
                .map(|row| row.get::<i64, _>("next_attempt_ms"))
        }};
    }
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => read!(sqlite.pool()),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => read!(pg.pool()),
    }
}

/// A walk that spent its day pauses until the day ends: the step that ran
/// into the refusal records what it read before it, and the next claim,
/// refused at once, is given back with its retry at the window's end.
#[tokio::test]
async fn a_spent_walk_day_pauses_the_walk_until_the_day_ends() {
    let (api_base, mock) = mock_strava().await;
    // 40 a day: a background share of 10, half of it, 5, for the walk.
    let resources = server(&api_base, 40).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    let starts: Vec<i64> = (1..=7).map(|day| now - day * DAY).collect();
    store_rides(&resources, user_id, tenant, SESSION, &starts).await;
    open_window(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let asked = ask_detail_walk(
        &runtime,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        since(now - 10 * DAY),
    )
    .await;
    assert_eq!(
        asked,
        DetailWalkAsked::Started { units_total: 7 },
        "the seven rides of the window are owed"
    );

    // The first step reads the day's five and records them.
    let deadline = Instant::now() + WALK_DEADLINE;
    while walk_row(&resources, user_id, tenant)
        .await
        .is_none_or(|row| row.progress.units_done < 5)
    {
        assert!(Instant::now() < deadline, "the first step did not record");
        sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        mock.detail_reads().len(),
        5,
        "bug: the walk read past its day's five"
    );
    // The next claim meets the spent day and gives the row back. A sweep
    // that finds the ask's own run still holding the pair hands its claim
    // back untouched, so the sweep is asked until one runs.
    let paused = loop {
        age_job_rows(&resources, user_id).await;
        resume_backfill_jobs(Arc::clone(&runtime)).await.unwrap();
        sleep(Duration::from_millis(50)).await;
        if let Some(at) = next_attempt_ms(&resources, user_id)
            .await
            .filter(|at| *at > 0)
        {
            break at;
        }
        assert!(
            Instant::now() < deadline,
            "the refused claim was not given back"
        );
    };
    let day_ms = DAY * 1_000;
    let window_end = (Utc::now().timestamp_millis() / day_ms + 1) * day_ms;
    assert!(
        (window_end - 2_000..=window_end + 5_000).contains(&paused),
        "bug: a spent day's walk waits for the window's end ({window_end}), not a lease: {paused}"
    );
    assert_eq!(
        mock.detail_reads().len(),
        5,
        "bug: the refused claim read past the spent day"
    );
    let row = walk_row(&resources, user_id, tenant).await.unwrap();
    assert_eq!(
        row.state,
        BackfillJobState::Owed,
        "bug: a paused walk ended"
    );
}

/// An ask while a walk is owed widens its floor and recounts its total from
/// what is unread below its cursor; the walk then reads down to the new
/// floor. While it runs, its progress is pending with an ETA at the budget's
/// pace.
#[tokio::test]
async fn an_ask_while_owed_widens_the_walk_and_it_walks_the_new_floor() {
    let (api_base, mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    let starts = [now - 10 * DAY, now - 20 * DAY, now - 40 * DAY];
    store_rides(&resources, user_id, tenant, SESSION, &starts).await;
    open_window(&resources).await;

    let gate = mock.hold_next_read();
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let first = ask_detail_walk(
        &runtime,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        since(now - 15 * DAY),
    )
    .await;
    assert_eq!(
        first,
        DetailWalkAsked::Started { units_total: 1 },
        "the one ride above the first floor is owed"
    );
    mock.read_held().await;

    let pending = detail_walk_pending(&runtime, user_id, tenant, oauth_providers::STRAVA)
        .await
        .unwrap();
    assert_eq!(
        pending,
        Some(Pending::DetailWalkRunning {
            units_done: 0,
            units_total: Some(1),
            eta_secs: Some(1_728),
        }),
        "bug: one ride left at the budget's 50 a day is 1728 s"
    );

    let wider = ask_detail_walk(
        &runtime,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        since(now - 50 * DAY),
    )
    .await;
    assert_eq!(
        wider,
        DetailWalkAsked::Widened,
        "bug: an ask while one is owed records a second walk"
    );
    let row = walk_row(&resources, user_id, tenant).await.unwrap();
    assert_eq!(
        row.after_ts,
        Some(now - 50 * DAY),
        "bug: the owed walk's floor is lowered to the new ask's"
    );
    assert_eq!(
        row.progress.units_total,
        Some(3),
        "bug: the total is recounted over the widened window below the cursor"
    );

    gate.notify_one();
    let ended = walk_ended(&resources, user_id, tenant).await;
    assert_eq!(
        ended.state,
        BackfillJobState::Walked,
        "bug: the widened walk never reached its new floor"
    );
    assert_eq!(
        ended.progress.units_done, 3,
        "bug: the walk reads down to the widened floor"
    );
    let mut reads = mock.detail_reads();
    reads.sort_unstable();
    reads.dedup();
    assert_eq!(
        reads.len(),
        3,
        "bug: a ride below the first floor was never read"
    );
}

/// A window whose rides all carry their efforts owes no walk.
#[tokio::test]
async fn a_window_read_already_records_no_walk() {
    let (api_base, _mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let asked = ask_detail_walk(
        &runtime,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        since(now - 30 * DAY),
    )
    .await;
    assert_eq!(
        asked,
        DetailWalkAsked::NothingUnread,
        "bug: a window with no stored ride asked for a walk"
    );
    assert!(
        walk_row(&resources, user_id, tenant).await.is_none(),
        "bug: an empty window recorded a walk"
    );
    assert!(
        resources
            .common
            .repos
            .activity_backfill_jobs
            .backfill_job_state(
                user_id,
                tenant,
                oauth_providers::STRAVA,
                BackfillJobKind::DetailWalk
            )
            .await
            .unwrap()
            .is_none(),
        "bug: an empty window left a ledger row"
    );
}

/// Every detail walk's end the athlete was told: its backend and count.
#[derive(Default)]
struct Told {
    walks: Mutex<Vec<(String, usize)>>,
}

#[async_trait]
impl BackfillNotifier for Told {
    // A detail walk is asked by no conversation: nothing here reaches it.
    async fn push_backfill_complete(&self, _: BackfillCompletion<'_>) {}

    async fn push_history_walk_complete(&self, _: HistoryWalkCompletion<'_>) {}

    async fn push_detail_walk_complete(&self, completion: DetailWalkCompletion<'_>) {
        self.walks
            .lock()
            .unwrap()
            .push((completion.provider.to_owned(), completion.activity_count));
    }

    async fn push_provider_reauth(&self, _: Uuid, _: TenantId, _: &str, _: &str) {}

    async fn push_reauth_to_linked_channels(
        &self,
        _: Uuid,
        _: TenantId,
        _: &str,
        _: ReauthRetry,
    ) -> usize {
        0
    }
}

/// `resources` telling the athlete through `told`.
fn telling(resources: &Arc<ServerContext>, told: &Arc<Told>) -> Arc<ServerContext> {
    let mut context = (**resources).clone();
    let notifier: Arc<dyn BackfillNotifier> = told.clone();
    context.mcp.backfill_notifier = Some(notifier);
    Arc::new(context)
}

/// A walk that stored efforts tells the athlete once, when it ends, how many
/// activities it read them for, over however many steps it took; a walk
/// that stored none tells nothing.
#[tokio::test]
async fn a_walk_that_stored_efforts_tells_its_end_once() {
    let (api_base, mock) = mock_strava().await;
    let told = Arc::new(Told::default());
    let resources = telling(&server(&api_base, 400).await, &told);
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    // Twelve rides: two steps of the walk.
    let starts: Vec<i64> = (1..=12).map(|day| now - day * DAY).collect();
    store_rides(&resources, user_id, tenant, SESSION, &starts).await;
    open_window(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let asked = ask_detail_walk(
        &runtime,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        since(now - 20 * DAY),
    )
    .await;
    assert_eq!(
        asked,
        DetailWalkAsked::Started { units_total: 12 },
        "the twelve rides of the window are owed"
    );
    let ended = walk_ended(&resources, user_id, tenant).await;
    assert_eq!(
        ended.progress.rows_landed, 12,
        "the walk stored every ride's efforts"
    );
    assert_eq!(
        *told.walks.lock().unwrap(),
        vec![(oauth_providers::STRAVA.to_owned(), 12)],
        "bug: the walk's end is told once, counting what every step stored"
    );

    // A walk over rides the API cannot read stores nothing, and says nothing.
    let (other, other_tenant) = strava_athlete(&resources).await;
    let unreadable = [now - 3 * DAY];
    mock.missing.lock().unwrap().insert(unreadable[0]);
    store_rides(&resources, other, other_tenant, SESSION, &unreadable).await;
    let asked = ask_detail_walk(
        &runtime,
        other,
        other_tenant,
        oauth_providers::STRAVA,
        since(now - 20 * DAY),
    )
    .await;
    assert_eq!(
        asked,
        DetailWalkAsked::Started { units_total: 1 },
        "the other athlete's one ride is owed"
    );
    let ended = walk_ended(&resources, other, other_tenant).await;
    assert_eq!(
        ended.progress.rows_landed, 0,
        "the unreadable ride is not stored"
    );
    assert_eq!(
        told.walks.lock().unwrap().len(),
        1,
        "bug: a walk that stored nothing told the athlete it ended"
    );
}

/// A walk that walked past a ride Strava no longer has tells what it stored,
/// never what it walked: two of its three rides.
#[tokio::test]
async fn a_walk_that_walked_past_a_ride_tells_only_what_it_stored() {
    let (api_base, mock) = mock_strava().await;
    let told = Arc::new(Told::default());
    let resources = telling(&server(&api_base, 400).await, &told);
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    let starts = [now - 2 * DAY, now - 3 * DAY, now - 4 * DAY];
    mock.missing.lock().unwrap().insert(starts[1]);
    store_rides(&resources, user_id, tenant, SESSION, &starts).await;
    open_window(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let asked = ask_detail_walk(
        &runtime,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        since(now - 10 * DAY),
    )
    .await;
    assert_eq!(
        asked,
        DetailWalkAsked::Started { units_total: 3 },
        "the three rides are owed"
    );
    let ended = walk_ended(&resources, user_id, tenant).await;
    assert_eq!(
        (
            ended.state,
            ended.progress.units_done,
            ended.progress.rows_landed
        ),
        (BackfillJobState::Walked, 3, 2),
        "the walk walked three rides and stored two"
    );
    assert_eq!(
        *told.walks.lock().unwrap(),
        vec![(oauth_providers::STRAVA.to_owned(), 2)],
        "bug: the post counts a ride the walk walked past as read"
    );
}

/// The session lists a ride on its day alone, the grant at its real start:
/// two rows of one ride that start in different seconds. The walk counts and
/// reads it once, and a later ask finds nothing left to read.
#[tokio::test]
async fn copies_of_one_ride_listed_at_different_starts_are_read_once() {
    let (api_base, mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    let real_start = now - 10 * DAY;
    let day_start = real_start - real_start.rem_euclid(DAY);
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    runtime
        .live_activities()
        .ingest(
            user_id,
            tenant,
            SESSION,
            &[ride_listed_at(SESSION, real_start, day_start)],
            Utc::now(),
        )
        .await
        .unwrap();
    store_rides(
        &resources,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        &[real_start],
    )
    .await;
    open_window(&resources).await;

    let asked = ask_detail_walk(
        &runtime,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        since(now - 30 * DAY),
    )
    .await;
    assert_eq!(
        asked,
        DetailWalkAsked::Started { units_total: 1 },
        "bug: the session's and the grant's copies of one ride are counted twice"
    );
    let ended = walk_ended(&resources, user_id, tenant).await;
    assert_eq!(
        ended.progress.units_done, 1,
        "bug: the walk read the ride once per copy"
    );
    assert_eq!(
        mock.detail_reads(),
        vec![real_start],
        "bug: the ride is read through the grant once"
    );
    assert_eq!(
        ask_detail_walk(
            &runtime,
            user_id,
            tenant,
            oauth_providers::STRAVA,
            since(now - 30 * DAY),
        )
        .await,
        DetailWalkAsked::NothingUnread,
        "bug: a ride whose grant copy carries its efforts reads as unread through its session copy"
    );
}

/// An athlete whose walk over `[now - 10 days, top)` of `sport` ended
/// walked, having read two rides and failed on the third (`404`).
struct WalkedOnce {
    resources: Arc<ServerContext>,
    mock: Arc<MockStrava>,
    user_id: Uuid,
    tenant: TenantId,
    now: i64,
    ask: DetailWalkAsk<'static>,
}

async fn walked_once(sport: Option<&'static str>) -> WalkedOnce {
    let (api_base, mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    let starts = [now - 5 * DAY, now - 6 * DAY, now - 7 * DAY];
    mock.missing.lock().unwrap().insert(starts[1]);
    store_rides(&resources, user_id, tenant, SESSION, &starts).await;
    open_window(&resources).await;
    // A walk tried what was stored before the second it was asked in.
    sleep(Duration::from_millis(1_100)).await;
    let ask = DetailWalkAsk {
        after_ts: now - 10 * DAY,
        before_ts: now,
        sport,
    };
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert_eq!(
        ask_detail_walk(&runtime, user_id, tenant, oauth_providers::STRAVA, ask).await,
        DetailWalkAsked::Started { units_total: 3 },
        "the three rides are owed"
    );
    let ended = walk_ended(&resources, user_id, tenant).await;
    assert_eq!(
        ended.state,
        BackfillJobState::Walked,
        "the walk ended walked"
    );
    assert_eq!(mock.detail_reads().len(), 3, "each ride was read once");
    WalkedOnce {
        resources,
        mock,
        user_id,
        tenant,
        now,
        ask,
    }
}

impl WalkedOnce {
    async fn ask(&self, ask: DetailWalkAsk<'_>) -> DetailWalkAsked {
        let runtime: Arc<dyn ToolRuntime> = self.resources.clone();
        ask_detail_walk(
            &runtime,
            self.user_id,
            self.tenant,
            oauth_providers::STRAVA,
            ask,
        )
        .await
    }

    async fn state(&self) -> Option<BackfillJobState> {
        walk_row(&self.resources, self.user_id, self.tenant)
            .await
            .map(|row| row.state)
    }
}

/// A walk that ended walked has tried every ride of its window: an ask
/// over that window within a week records no new walk, which would only
/// spend the athlete's day re-reading the rides the backend could not read.
#[tokio::test]
async fn a_re_ask_within_a_week_of_a_walk_that_ended_reads_nothing_again() {
    let walked = walked_once(None).await;

    let asked = walked.ask(walked.ask).await;
    assert_eq!(
        asked,
        DetailWalkAsked::RecentlyWalked { unreadable: 1 },
        "bug: a re-ask over a window walked a moment ago records a fresh walk"
    );
    let narrower = DetailWalkAsk {
        after_ts: walked.now - 9 * DAY,
        before_ts: walked.now - DAY,
        sport: Some("Ride"),
    };
    assert_eq!(
        walked.ask(narrower).await,
        DetailWalkAsked::RecentlyWalked { unreadable: 1 },
        "bug: a narrower window of one sport inside the walked one records a fresh walk"
    );
    assert_eq!(
        walked.state().await,
        Some(BackfillJobState::Walked),
        "bug: the ended walk was replaced by an owed one"
    );
    let runtime: Arc<dyn ToolRuntime> = walked.resources.clone();
    resume_backfill_jobs(runtime).await.unwrap();
    sleep(Duration::from_millis(200)).await;
    assert_eq!(
        walked.mock.detail_reads().len(),
        3,
        "bug: the ride the backend could not read was read again"
    );
}

/// Set `assignment` on the athlete's detail walk row.
pub(crate) async fn alter_walk_row(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    assignment: &str,
) {
    let sql = format!(
        "UPDATE activity_backfill_jobs SET {assignment} \
         WHERE user_id = $1 AND kind = 'detail_walk'"
    );
    macro_rules! alter {
        ($pool:expr) => {{
            let altered = sqlx::query(&sql)
                .bind(user_id.to_string())
                .execute($pool)
                .await
                .unwrap();
            assert_eq!(altered.rows_affected(), 1, "the walk row is altered");
        }};
    }
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => alter!(sqlite.pool()),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => alter!(pg.pool()),
    }
}

/// A walk that ended over a week ago stands no more: a ride deleted or made
/// private since may read now, so the window is walked afresh.
#[tokio::test]
async fn a_walk_that_ended_over_a_week_ago_is_walked_afresh() {
    let walked = walked_once(None).await;
    let week_and_a_day_ms = 8 * DAY * 1_000;
    alter_walk_row(
        &walked.resources,
        walked.user_id,
        &format!("ended_at_ms = ended_at_ms - {week_and_a_day_ms}"),
    )
    .await;

    assert_eq!(
        walked.ask(walked.ask).await,
        DetailWalkAsked::Started { units_total: 1 },
        "bug: a walk that ended over a week ago still stands for its window"
    );
    assert_eq!(walked.state().await, Some(BackfillJobState::Owed));
}

/// A window reaching below the walked one's floor, where no ride is stored,
/// holds nothing the walk did not try: the walk stands for it. A ride stored
/// below that floor is one no walk tried: a fresh walk reads it alone, never
/// the ride the ended walk could not read.
#[tokio::test]
async fn a_ride_below_the_walked_floor_is_walked_alone() {
    let walked = walked_once(None).await;
    let lower = DetailWalkAsk {
        after_ts: walked.now - 20 * DAY,
        ..walked.ask
    };
    assert_eq!(
        walked.ask(lower).await,
        DetailWalkAsked::RecentlyWalked { unreadable: 1 },
        "bug: a window whose only unread ride the walk tried reads as owing a walk"
    );

    let below = walked.now - 15 * DAY;
    store_rides(
        &walked.resources,
        walked.user_id,
        walked.tenant,
        SESSION,
        &[below],
    )
    .await;
    assert_eq!(
        walked.ask(lower).await,
        DetailWalkAsked::Started { units_total: 1 },
        "bug: the fresh walk counts the ride the ended walk could not read"
    );
    walk_ended(&walked.resources, walked.user_id, walked.tenant).await;
    assert_eq!(
        walked.mock.detail_reads()[3..],
        [below],
        "bug: the fresh walk read again what the ended walk tried"
    );
}

/// The walk was asked a day ago, so its window's top is a day short of the
/// ask's: a default window ends at the athlete's next midnight, which moves
/// up a day every day.
async fn walked_a_day_ago() -> WalkedOnce {
    let walked = walked_once(None).await;
    alter_walk_row(
        &walked.resources,
        walked.user_id,
        &format!("window_top_ts = window_top_ts - {DAY}"),
    )
    .await;
    walked
}

/// A window reaching above the walked one's top, where no ride is unread,
/// holds nothing the walk did not try: the walk still stands for it, and
/// the ride it could not read is not read again.
#[tokio::test]
async fn a_window_above_the_walked_top_with_nothing_unread_there_stands_walked() {
    let walked = walked_a_day_ago().await;

    assert_eq!(
        walked.ask(walked.ask).await,
        DetailWalkAsked::RecentlyWalked { unreadable: 1 },
        "bug: a window whose top moved a day records a fresh walk"
    );
    assert_eq!(
        walked.state().await,
        Some(BackfillJobState::Walked),
        "bug: the ended walk was replaced by an owed one"
    );
    let runtime: Arc<dyn ToolRuntime> = walked.resources.clone();
    resume_backfill_jobs(runtime).await.unwrap();
    sleep(Duration::from_millis(200)).await;
    assert_eq!(
        walked.mock.detail_reads().len(),
        3,
        "bug: the ride the backend could not read was read again"
    );
}

/// A ride unread above the walked one's top is one no walk tried: the
/// window is walked afresh.
#[tokio::test]
async fn a_ride_unread_above_the_walked_top_is_walked_afresh() {
    let walked = walked_a_day_ago().await;
    store_rides(
        &walked.resources,
        walked.user_id,
        walked.tenant,
        SESSION,
        &[walked.now - DAY / 2],
    )
    .await;

    assert_eq!(
        walked.ask(walked.ask).await,
        DetailWalkAsked::Started { units_total: 1 },
        "bug: the fresh walk counts the ride the ended walk could not read"
    );
    walk_ended(&walked.resources, walked.user_id, walked.tenant).await;
    assert_eq!(
        walked.mock.detail_reads()[3..],
        [walked.now - DAY / 2],
        "bug: the fresh walk read again what the ended walk tried"
    );
}

/// A ride that started after the walk ended, below the top its window was
/// asked with (a default window's top is the athlete's next midnight), is
/// one the walk never saw: the window is walked afresh.
#[tokio::test]
async fn a_ride_started_after_the_walk_ended_is_walked_afresh() {
    let walked = walked_once(None).await;
    let two_days_ms = 2 * DAY * 1_000;
    alter_walk_row(
        &walked.resources,
        walked.user_id,
        &format!("ended_at_ms = ended_at_ms - {two_days_ms}"),
    )
    .await;
    store_rides(
        &walked.resources,
        walked.user_id,
        walked.tenant,
        SESSION,
        &[walked.now - DAY],
    )
    .await;

    assert_eq!(
        walked.ask(walked.ask).await,
        DetailWalkAsked::Started { units_total: 1 },
        "bug: a ride started after the walk ended reads as walked, or the fresh walk counts the ride the ended one could not read"
    );
}

/// A ride that started inside the walked window, before the walk ended, but
/// reached the store only after the walk was asked, is one the walk never
/// saw: a fresh walk reads it, and never reports it unreadable.
#[tokio::test]
async fn a_ride_stored_after_the_walk_was_asked_is_walked_not_unreadable() {
    let walked = walked_once(None).await;
    // The store keeps a row's first write to the second on SQLite: one past
    // the ask's.
    sleep(Duration::from_millis(1_100)).await;
    let late = walked.now - 4 * DAY;
    store_rides(
        &walked.resources,
        walked.user_id,
        walked.tenant,
        SESSION,
        &[late],
    )
    .await;

    assert_eq!(
        walked.ask(walked.ask).await,
        DetailWalkAsked::Started { units_total: 1 },
        "bug: a ride synced after the walk was asked reads as one it could not read"
    );
    walk_ended(&walked.resources, walked.user_id, walked.tenant).await;
    assert_eq!(
        walked.mock.detail_reads()[3..],
        [late],
        "bug: the late ride was not read, or the unreadable one was read again"
    );
}

/// A walk of one sport never tried the others: an ask for every sport is
/// walked afresh.
#[tokio::test]
async fn a_walk_of_one_sport_does_not_stand_for_every_sport() {
    let walked = walked_once(Some("Ride")).await;
    let every_sport = DetailWalkAsk {
        sport: None,
        ..walked.ask
    };
    assert_eq!(
        walked.ask(every_sport).await,
        DetailWalkAsked::Started { units_total: 1 },
        "bug: a walk of rides stands for every sport"
    );
}

/// Only a walk that reached its floor tried every ride of its window: one
/// that ended any other way stands for nothing, however lately it ended.
#[tokio::test]
async fn a_walk_that_ended_incomplete_does_not_stand_for_its_window() {
    let walked = walked_once(None).await;
    alter_walk_row(&walked.resources, walked.user_id, "outcome = 'incomplete'").await;
    assert_eq!(walked.state().await, Some(BackfillJobState::Incomplete));

    assert_eq!(
        walked.ask(walked.ask).await,
        DetailWalkAsked::Started { units_total: 1 },
        "bug: a walk that ended incomplete stands for its window"
    );
}
