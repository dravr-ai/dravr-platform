// ABOUTME: The tag walk: Strava's own activity list paged for the tags the newest page alone carries, into rows already stored
// ABOUTME: One page a call, from the first, within the session walk's burst, never overwriting a tag read with none, never creating a row

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tag walks (carnet#1622).
//
// The session reader is a loopback stand-in (a test double, per the repo's mock
// rule): its `/api/activity-search` serves the pages the test gives it, in
// pages of twenty, and records every request it sees.
#![cfg(all(
    feature = "tools-data",
    feature = "provider-strava",
    feature = "provider-garmin",
    feature = "health-sync"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers::history_walk_ledger::{end_history_walk, owe_history_walk};
use crate::helpers::sciotte_mock::seed_sciotte_session;
use crate::history_walk_api_test::{open_window, with_window};
use crate::history_walk_test::{age_job_rows, paused_retries_due};

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::models::{
    Activity, ActivityBuilder, ActivityTag, ConnectionType, SportType, TagSource, TaggedBy,
    TenantId,
};
use pierre_database::repositories::{BackfillJobKind, BackfillJobState};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::activity_backfill::tag_walk::{ask_tag_walk, TagWalkAsked};
use pierre_tool_runtime::activity_backfill_resume::resume_backfill_jobs;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::time;
use uuid::Uuid;

/// The key the athlete's session files its rows under.
const SESSION: &str = "sciotte";

/// The key the athlete's Strava grant files its rows under.
const GRANT: &str = "strava";

/// How long a walk gets to end before the test gives up.
const DEADLINE: Duration = Duration::from_secs(30);

/// Rows one stand-in page holds.
const PAGE_ROWS: usize = 20;

/// What the session reader stand-in serves and saw.
#[derive(Default)]
struct MockSessionReader {
    /// The search pages, page one first; a page past them is empty.
    pages: Mutex<Vec<Vec<Value>>>,
    /// The pages it answers `head_complete: false` for, once each.
    short_once: Mutex<HashSet<u32>>,
    /// Every search request's `(page, limit)`, in order.
    searches: Mutex<Vec<(u32, Option<u32>)>>,
}

impl MockSessionReader {
    fn searched_pages(&self) -> Vec<u32> {
        self.searches
            .lock()
            .unwrap()
            .iter()
            .map(|(page, _)| *page)
            .collect()
    }
}

/// `GET /api/activity-search`.
async fn search(
    State(mock): State<Arc<MockSessionReader>>,
    Query(query): Query<BTreeMap<String, String>>,
) -> Json<Value> {
    let page: u32 = query.get("page").and_then(|p| p.parse().ok()).unwrap_or(1);
    let limit = query.get("limit").and_then(|l| l.parse().ok());
    mock.searches.lock().unwrap().push((page, limit));
    let complete = !mock.short_once.lock().unwrap().remove(&page);
    let rows = mock
        .pages
        .lock()
        .unwrap()
        .get(usize::try_from(page).unwrap() - 1)
        .cloned()
        .unwrap_or_default();
    Json(json!({
        "count": rows.len(),
        "activities": rows,
        "head_complete": complete
    }))
}

async fn mock_session_reader(pages: Vec<Vec<Value>>) -> (String, Arc<MockSessionReader>) {
    let mock = Arc::new(MockSessionReader {
        pages: Mutex::new(pages),
        ..MockSessionReader::default()
    });
    let app = Router::new()
        .route(
            "/auth/import-session",
            post(|| async { Json(json!({ "session_id": "cap-verified-session" })) }),
        )
        .route("/api/activity-search", get(search))
        .with_state(Arc::clone(&mock));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), mock)
}

/// The id of run `n`, and its start.
fn run_id(n: usize) -> String {
    (10_000 + n).to_string()
}

fn run_start(n: usize) -> Timestamp {
    Timestamp::parse_from_rfc3339("2026-06-01T07:00:00Z").unwrap()
        - Elapsed::days(i64::try_from(n).unwrap())
}

/// Search page row `n`, as the service lists it, with the athlete's tag map
/// when `tags` is given.
fn listed(n: usize, tags: Option<Value>) -> Value {
    let mut row = json!({
        "id": run_id(n),
        "name": "Run",
        "sport_type": "run",
        "start_date": run_start(n).to_rfc3339(),
        "duration_seconds": 1_800,
        "provider": "strava"
    });
    if let Some(tags) = tags {
        row["tags"] = tags;
    }
    row
}

/// `rows` runs split into pages of twenty, every row tagged Race (Strava's
/// tag id 1) when `tagged`.
fn pages_of(rows: usize, tagged: bool) -> Vec<Vec<Value>> {
    let all: Vec<Value> = (0..rows)
        .map(|n| listed(n, tagged.then(|| json!({ "1": true }))))
        .collect();
    all.chunks(PAGE_ROWS).map(<[Value]>::to_vec).collect()
}

/// The athlete's stored run `n` under the session's key, with no tag read.
fn stored_run(n: usize) -> Activity {
    stored_run_under(n, SESSION)
}

/// Stored run `n` as the backend `key` stored it, with no tag read.
fn stored_run_under(n: usize, key: &str) -> Activity {
    ActivityBuilder::new(
        run_id(n),
        "Stored run",
        SportType::Run,
        run_start(n),
        1_800,
        key,
    )
    .build()
}

/// A server reaching the session reader stand-in, and an athlete holding the live
/// session, whose history walk ended and whose store holds `stored` runs.
async fn athlete(reader_url: &str, stored: usize) -> (Arc<ServerContext>, Uuid, TenantId) {
    let mut config = common::test_server_config_with_sciotte(reader_url);
    common::set_server_app(&mut config, "strava", "walk_client", "walk_secret");
    let resources = common::create_test_server_resources_with_config(config)
        .await
        .unwrap();
    let resources = with_window(&resources, 400);
    let (user_id, _user, tenant) = common::create_test_user_with_plan(
        &resources.agent.database,
        &format!("tag-walk-{}@example.com", Uuid::new_v4()),
        "starter",
    )
    .await
    .unwrap();
    seed_sciotte_session(&resources, user_id, tenant).await;
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, SESSION, &ConnectionType::Manual, None)
        .await
        .unwrap();
    let rows: Vec<Activity> = (0..stored).map(stored_run).collect();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(user_id, &tenant, SESSION, &rows)
        .await
        .unwrap();
    open_window(&resources).await;
    (resources, user_id, tenant)
}

/// End the athlete's history walk, as the walk that stored the history does.
async fn history_walked(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId) {
    let repos = &resources.common.repos;
    let lease = owe_history_walk(repos, user_id, tenant, SESSION).await;
    end_history_walk(repos, user_id, tenant, SESSION, &lease).await;
}

async fn state(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> Option<BackfillJobState> {
    resources
        .common
        .repos
        .activity_backfill_jobs
        .backfill_job_state(user_id, tenant, SESSION, BackfillJobKind::TagWalk)
        .await
        .unwrap()
}

/// Run the walk the sweep's way, a step a pass, until its row is no longer
/// owed.
async fn walk_ended(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> BackfillJobState {
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let deadline = Instant::now() + DEADLINE;
    loop {
        time::sleep(Duration::from_millis(50)).await;
        match state(resources, user_id, tenant).await {
            Some(BackfillJobState::Owed) => {}
            Some(ended) => return ended,
            None => panic!("the walk's row is gone"),
        }
        assert!(Instant::now() < deadline, "the walk did not end in time");
        paused_retries_due(resources, user_id).await;
        age_job_rows(resources, user_id).await;
        resume_backfill_jobs(runtime.clone()).await.unwrap();
    }
}

async fn read_run(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    n: usize,
) -> Option<Activity> {
    resources
        .common
        .repos
        .activity_cache
        .get_cached_activity(user_id, &tenant, SESSION, &run_id(n))
        .await
        .unwrap()
}

fn race() -> [TaggedBy; 1] {
    [TaggedBy::new(ActivityTag::Race, TagSource::ProviderTag)]
}

/// The walk reads the list from its first page, a page a call and bounded to
/// that page's rows, and fills the tags of every stored run on it, the runs
/// past the newest page included; it ends on the first empty page and is not
/// asked again for the connection.
#[tokio::test]
async fn a_walk_fills_the_tags_of_the_whole_stored_history() {
    let (url, reader) = mock_session_reader(pages_of(50, true)).await;
    let (resources, user_id, tenant) = athlete(&url, 50).await;
    history_walked(&resources, user_id, tenant).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();

    let asked = ask_tag_walk(&runtime, user_id, tenant, SESSION).await;
    assert_eq!(asked, TagWalkAsked::Started, "bug: the walk was not asked");
    assert_eq!(
        walk_ended(&resources, user_id, tenant).await,
        BackfillJobState::Walked,
        "bug: the walk did not end walked"
    );

    for n in [0, 19, 20, 39, 40, 49] {
        let run = read_run(&resources, user_id, tenant, n).await.unwrap();
        assert_eq!(
            run.tags(),
            Some(&race()[..]),
            "bug: run {n} past the newest page was not tagged"
        );
        assert_eq!(
            run.name(),
            "Stored run",
            "bug: the walk moved another field"
        );
    }
    assert_eq!(
        reader.searched_pages(),
        vec![1, 2, 3, 4],
        "bug: the walk did not read one page a call, in order, to the empty one"
    );
    assert!(
        reader
            .searches
            .lock()
            .unwrap()
            .iter()
            .all(|(_, limit)| *limit == Some(20)),
        "bug: a call was not bounded to one page of rows"
    );
    assert_eq!(
        ask_tag_walk(&runtime, user_id, tenant, SESSION).await,
        TagWalkAsked::Standing,
        "bug: an ended walk is asked again for the connection"
    );
}

/// A page whose runs the store holds none of ends the walk: the list is past
/// the stored history, and no row is created for what it lists.
#[tokio::test]
async fn a_walk_ends_past_the_stored_history_and_creates_no_row() {
    let (url, reader) = mock_session_reader(pages_of(60, true)).await;
    let (resources, user_id, tenant) = athlete(&url, 20).await;
    history_walked(&resources, user_id, tenant).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();

    assert_eq!(
        ask_tag_walk(&runtime, user_id, tenant, SESSION).await,
        TagWalkAsked::Started,
        "bug: the walk was not asked"
    );
    assert_eq!(
        walk_ended(&resources, user_id, tenant).await,
        BackfillJobState::Walked,
        "bug: the walk did not end walked"
    );

    assert_eq!(
        reader.searched_pages(),
        vec![1, 2],
        "bug: the walk read on past the stored history"
    );
    assert!(
        read_run(&resources, user_id, tenant, 20).await.is_none(),
        "bug: the walk created a row for a run the store never held"
    );
    assert_eq!(
        read_run(&resources, user_id, tenant, 5)
            .await
            .unwrap()
            .tags(),
        Some(&race()[..]),
        "bug: a stored run in the walked list was not tagged"
    );
}

/// A page row that carries no tag map never erases the tags the store holds,
/// and a row that carries one replaces them.
#[tokio::test]
async fn a_walk_never_replaces_a_stored_tag_read_with_none() {
    let mut rows = pages_of(3, false);
    rows[0][2] = listed(2, Some(json!({ "12": true })));
    let (url, _reader) = mock_session_reader(rows).await;
    let (resources, user_id, tenant) = athlete(&url, 3).await;
    let kept = ActivityBuilder::new(
        run_id(0),
        "Stored run",
        SportType::Run,
        run_start(0),
        1_800,
        SESSION,
    )
    .tags_opt(Some(race().to_vec()))
    .build();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(user_id, &tenant, SESSION, &[kept])
        .await
        .unwrap();
    history_walked(&resources, user_id, tenant).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();

    ask_tag_walk(&runtime, user_id, tenant, SESSION).await;
    walk_ended(&resources, user_id, tenant).await;

    assert_eq!(
        read_run(&resources, user_id, tenant, 0)
            .await
            .unwrap()
            .tags(),
        Some(&race()[..]),
        "bug: a list row without a tag map erased the stored tags"
    );
    assert_eq!(
        read_run(&resources, user_id, tenant, 2)
            .await
            .unwrap()
            .tags(),
        Some(&[TaggedBy::new(ActivityTag::Recovery, TagSource::ProviderTag)][..]),
        "bug: a list row's own tag read was not stored"
    );
}

/// A page the service did not read whole is read again at the same page and
/// never counted: the cursor does not move past it.
#[tokio::test]
async fn a_page_the_service_did_not_read_whole_is_read_again() {
    let (url, reader) = mock_session_reader(pages_of(40, true)).await;
    reader.short_once.lock().unwrap().insert(2);
    let (resources, user_id, tenant) = athlete(&url, 40).await;
    history_walked(&resources, user_id, tenant).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();

    ask_tag_walk(&runtime, user_id, tenant, SESSION).await;
    assert_eq!(
        walk_ended(&resources, user_id, tenant).await,
        BackfillJobState::Walked,
        "bug: the walk did not end walked after the re-read"
    );

    assert_eq!(
        reader.searched_pages(),
        vec![1, 2, 2, 3],
        "bug: the short page was skipped or the walk lost its place"
    );
    assert_eq!(
        read_run(&resources, user_id, tenant, 39)
            .await
            .unwrap()
            .tags(),
        Some(&race()[..]),
        "bug: the page read again was not tagged"
    );
}

/// The walk spends the session walk's burst, not more: of thirty pages, ten
/// are read in fifteen minutes, once each in order, and the walk waits for
/// its window, owing the eleventh.
#[tokio::test]
async fn a_walk_reads_no_more_than_the_session_burst() {
    let (url, reader) = mock_session_reader(pages_of(600, true)).await;
    let (resources, user_id, tenant) = athlete(&url, 600).await;
    history_walked(&resources, user_id, tenant).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();

    ask_tag_walk(&runtime, user_id, tenant, SESSION).await;
    let deadline = Instant::now() + DEADLINE;
    while reader.searched_pages().len() < 10 {
        assert!(
            Instant::now() < deadline,
            "bug: the walk read only {} pages",
            reader.searched_pages().len()
        );
        paused_retries_due(&resources, user_id).await;
        age_job_rows(&resources, user_id).await;
        resume_backfill_jobs(runtime.clone()).await.unwrap();
        time::sleep(Duration::from_millis(50)).await;
    }
    for _ in 0..12 {
        paused_retries_due(&resources, user_id).await;
        resume_backfill_jobs(runtime.clone()).await.unwrap();
        time::sleep(Duration::from_millis(100)).await;
    }

    assert_eq!(
        reader.searched_pages(),
        (1..=10).collect::<Vec<u32>>(),
        "bug: the walk did not read ten pages once each, in order, and wait"
    );
    assert_eq!(
        state(&resources, user_id, tenant).await,
        Some(BackfillJobState::Owed),
        "bug: the walk ended instead of waiting for its window"
    );
    let progress = resources
        .common
        .repos
        .activity_backfill_jobs
        .job_progress(user_id, tenant, SESSION, BackfillJobKind::TagWalk)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        progress.units_done, 10,
        "bug: the cursor is not at the page the budget refused"
    );
}

/// A walk is asked only once the connection's history walk has ended, and
/// only for a backend that is a counted session.
#[tokio::test]
async fn a_walk_is_asked_only_after_the_history_walk_and_only_for_a_session() {
    let (url, reader) = mock_session_reader(pages_of(20, true)).await;
    let (resources, user_id, tenant) = athlete(&url, 20).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();

    let lease = owe_history_walk(&resources.common.repos, user_id, tenant, SESSION).await;
    assert_eq!(
        ask_tag_walk(&runtime, user_id, tenant, SESSION).await,
        TagWalkAsked::HistoryUnfinished,
        "bug: a walk was asked before the history was stored"
    );
    assert_eq!(
        ask_tag_walk(&runtime, user_id, tenant, "strava").await,
        TagWalkAsked::NotAsked,
        "bug: a walk was asked for an API grant"
    );
    assert!(
        state(&resources, user_id, tenant).await.is_none(),
        "bug: a walk row was recorded before the history ended"
    );
    assert_eq!(
        reader.searched_pages(),
        Vec::<u32>::new(),
        "bug: the service was asked before the history ended"
    );

    end_history_walk(&resources.common.repos, user_id, tenant, SESSION, &lease).await;
    assert_eq!(
        ask_tag_walk(&runtime, user_id, tenant, SESSION).await,
        TagWalkAsked::Started,
        "bug: the walk was not asked once the history ended"
    );
    assert_eq!(
        ask_tag_walk(&runtime, user_id, tenant, SESSION).await,
        TagWalkAsked::Standing,
        "bug: a second walk was recorded while one stands"
    );
}

/// A connection whose history came from its first sync or an import has no
/// history walk row: nothing is walking it, so its tag walk is asked all the
/// same.
#[tokio::test]
async fn a_connection_with_no_history_walk_row_still_gets_its_tag_walk() {
    let (url, _reader) = mock_session_reader(pages_of(20, true)).await;
    let (resources, user_id, tenant) = athlete(&url, 20).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert!(
        resources
            .common
            .repos
            .activity_backfill_jobs
            .history_walk_row(user_id, tenant, SESSION)
            .await
            .unwrap()
            .is_none(),
        "the fixture holds no history walk row"
    );

    assert_eq!(
        ask_tag_walk(&runtime, user_id, tenant, SESSION).await,
        TagWalkAsked::Started,
        "bug: a connection without a history walk row was never asked"
    );
    assert_eq!(
        walk_ended(&resources, user_id, tenant).await,
        BackfillJobState::Walked,
        "bug: the walk did not end walked"
    );
    assert_eq!(
        read_run(&resources, user_id, tenant, 7)
            .await
            .unwrap()
            .tags(),
        Some(&race()[..]),
        "bug: the run was not tagged"
    );
}

/// The connection's history is held under every key it spans: the session's
/// and the Strava grant's. A page whose runs sit under the grant is not past
/// the stored history, and its runs are tagged where they are stored.
#[tokio::test]
async fn a_walk_amends_a_history_split_across_the_session_and_the_grant() {
    let (url, reader) = mock_session_reader(pages_of(40, true)).await;
    let (resources, user_id, tenant) = athlete(&url, 0).await;
    history_walked(&resources, user_id, tenant).await;
    let cache = &resources.common.repos.activity_cache;
    // Page one's runs sit under the grant alone, page two's under the session.
    let under_grant: Vec<Activity> = (0..20).map(|n| stored_run_under(n, GRANT)).collect();
    let under_session: Vec<Activity> = (20..40).map(stored_run).collect();
    cache
        .upsert_activities(user_id, &tenant, GRANT, &under_grant)
        .await
        .unwrap();
    cache
        .upsert_activities(user_id, &tenant, SESSION, &under_session)
        .await
        .unwrap();
    let runtime: Arc<dyn ToolRuntime> = resources.clone();

    assert_eq!(
        ask_tag_walk(&runtime, user_id, tenant, SESSION).await,
        TagWalkAsked::Started,
        "bug: the walk was not asked"
    );
    assert_eq!(
        walk_ended(&resources, user_id, tenant).await,
        BackfillJobState::Walked,
        "bug: the walk did not end walked"
    );

    assert_eq!(
        reader.searched_pages(),
        vec![1, 2, 3],
        "bug: a page whose runs sit under the grant ended the walk"
    );
    for n in [0, 19] {
        let run = cache
            .get_cached_activity(user_id, &tenant, GRANT, &run_id(n))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(run.tags(), Some(&race()[..]), "bug: grant run {n} untagged");
    }
    for n in [20, 39] {
        let run = read_run(&resources, user_id, tenant, n).await.unwrap();
        assert_eq!(
            run.tags(),
            Some(&race()[..]),
            "bug: session run {n} untagged"
        );
    }
}
