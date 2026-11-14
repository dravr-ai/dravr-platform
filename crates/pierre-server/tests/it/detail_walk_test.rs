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
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use sqlx::Row;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

/// Seconds in a day.
const DAY: i64 = 86_400;

/// How long a walk gets to end before the test gives up.
const WALK_DEADLINE: Duration = Duration::from_secs(30);

/// The key the athlete's session files its rides under.
const SESSION: &str = "sciotte";

/// What the Strava stand-in answers and saw.
#[derive(Default)]
struct MockStrava {
    /// Every detail read's id, in the order they came.
    details: Mutex<Vec<i64>>,
    /// The ids it answers `404` for.
    missing: Mutex<HashSet<i64>>,
    /// The gate the next detail read waits on once it is counted.
    hold: Mutex<Option<Arc<Notify>>>,
    /// Raised when a detail read reached the gate.
    held: Notify,
}

impl MockStrava {
    fn detail_reads(&self) -> Vec<i64> {
        self.details.lock().unwrap().clone()
    }

    /// Hold the next detail read, once counted, until the returned gate is
    /// notified.
    fn hold_next_read(&self) -> Arc<Notify> {
        let gate = Arc::new(Notify::new());
        *self.hold.lock().unwrap() = Some(Arc::clone(&gate));
        gate
    }

    async fn read_held(&self) {
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

async fn mock_strava() -> (String, Arc<MockStrava>) {
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
fn ride(key: &str, start: i64) -> Activity {
    let built = ActivityBuilder::new(
        start.to_string(),
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
async fn store_rides(
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
async fn rows_under(
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
async fn walk_row(
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

/// Run the walk the sweep's way, one step a pass, in a fresh window each
/// pass, until its row ends.
async fn walk_ended(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> DetailWalkRow {
    let deadline = Instant::now() + WALK_DEADLINE;
    loop {
        let row = walk_row(resources, user_id, tenant).await;
        if let Some(row) = row
            .as_ref()
            .filter(|row| row.state != BackfillJobState::Owed)
        {
            return row.clone();
        }
        assert!(
            Instant::now() < deadline,
            "the walk did not end within {WALK_DEADLINE:?} (row: {row:?})"
        );
        open_window(resources).await;
        paused_retries_due(resources, user_id).await;
        age_job_rows(resources, user_id).await;
        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        resume_backfill_jobs(runtime).await.unwrap();
        sleep(Duration::from_millis(50)).await;
    }
}

/// The window `[after, now)` of every sport.
fn since(after: i64) -> DetailWalkAsk<'static> {
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
    assert_eq!(asked, DetailWalkAsked::Started { units_total: 7 });

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
    assert_eq!(first, DetailWalkAsked::Started { units_total: 1 });
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
    assert_eq!(ended.state, BackfillJobState::Walked);
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
    assert_eq!(asked, DetailWalkAsked::NothingUnread);
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
