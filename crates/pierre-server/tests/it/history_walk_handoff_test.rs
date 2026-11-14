// ABOUTME: A Strava walk reads the usr/pwd session 36 months a step, then pages the API grant below the oldest row it stored
// ABOUTME: One row through both phases: progress covers both, a restart resumes the API phase, the end is told once

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The hand-off from a session's walk to its API grant (carnet#1482).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// A Strava athlete holding the usr/pwd session and the API grant is walked
// through the session first, 36 months a request. Once a session step finds
// nothing, the walk goes on through the grant, a year a window, strictly
// below the oldest row the session stored, down to the account's creation
// the grant's profile names: what the scraper never listed is still the
// API's. The row keeps the session's key and stays owed through both phases,
// its progress covers both, a restart resumes the API phase rather than the
// session, and the athlete is told once, at the very end. A session walked
// with no grant beside it ends there, and is reopened for the API phase when
// a grant connects later.
//
// The scraper and Strava are loopback stand-ins (test doubles, per the repo's
// mock rule) that serve the history inside each read's bounds and count
// every request.
#![cfg(all(
    feature = "provider-strava",
    feature = "provider-garmin",
    feature = "health-sync"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers::sciotte_mock::seed_scrape_session;
use crate::history_walk_api_test::{
    hold_strava_grant, mock_strava, open_window, rows_under, strava_athlete, with_window,
    MockStrava,
};
use crate::history_walk_test::{age_job_rows, paused_retries_due};

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::extract::RawQuery;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};
use pierre_config::environment::ProviderEndpointOverride;
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::errors::AppResult;
use pierre_core::models::{
    Activity, ActivityBuilder, ActivityGearKey, AttributionSource, ConnectionType, DataSource,
    DeviceType, GearAttribution, SportType, StoredDailyActivity, TenantId, TimeSeriesData,
};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::{
    ActivityStreamsWrite, BackfillJobClaim, BackfillJobKind, BackfillJobState, JobLeaseState,
    JobProgress, PurgeReason, StreamOrigin, WellnessWriteMode,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_services::live_activities::{LiveActivityStore, LiveIngest};
use pierre_services::oauth_flow::OAuthService;
use pierre_services::provider_revocation::DisconnectReason;
use pierre_tool_runtime::activity_backfill::history_walk::start_history_walk;
use pierre_tool_runtime::activity_backfill::{BACKFILL_JOB_LEASE, MAX_BACKFILL_ATTEMPTS};
use pierre_tool_runtime::activity_backfill_resume::resume_backfill_jobs;
use pierre_tool_runtime::runtime::{
    BackfillCompletion, BackfillNotifier, DetailWalkCompletion, HistoryWalkCompletion, ReauthRetry,
    ToolRuntime, WalkTrigger,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

/// Seconds in a day.
const DAY: i64 = 86_400;

/// Seconds in a year, as the test counts them.
const YEAR: i64 = 365 * DAY;

/// How long a walk gets to end before the test gives up.
const WALK_DEADLINE: Duration = Duration::from_secs(30);

/// The session stand-in: the starts (unix seconds) its account lists, and
/// every activity read's query.
#[derive(Default)]
struct MockSession {
    starts: Mutex<Vec<i64>>,
    reads: Mutex<Vec<String>>,
}

impl MockSession {
    fn read_count(&self) -> usize {
        self.reads.lock().unwrap().len()
    }
}

/// The epoch-seconds bound `name` an activities read's query names.
fn query_bound(query: &str, name: &str) -> Option<i64> {
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix(&format!("{name}=")))
        .and_then(|value| value.parse().ok())
}

/// The scraper's JSON for the Strava ride starting at `start`, whose id
/// tells it apart from the API's rides (whose id is their start).
fn session_ride(start: i64) -> Value {
    json!({
        "id": format!("9{start}"),
        "name": "Sortie",
        "sport_type": "ride",
        "start_date": Utc.timestamp_opt(start, 0).unwrap().to_rfc3339(),
        "duration_seconds": 3600,
        "provider": "strava",
        "distance_meters": 20000.0
    })
}

/// A stand-in for the scraper service over a Strava account listing
/// `starts`, served within each read's `after` (inclusive) and `before`.
async fn spawn_session(starts: Vec<i64>) -> (String, Arc<MockSession>) {
    let mock = Arc::new(MockSession::default());
    *mock.starts.lock().unwrap() = starts;
    let listed = Arc::clone(&mock);
    let app = Router::new()
        .route(
            "/auth/import-session",
            post(|Json(body): Json<Value>| async move {
                Json(json!({ "session_id": body["session"]["session_id"] }))
            }),
        )
        .route(
            "/api/athlete",
            get(|| async { Json(json!({ "display_name": "Ses Sion" })) }),
        )
        .route(
            "/api/activities",
            get(move |RawQuery(query): RawQuery| {
                let mock = Arc::clone(&listed);
                async move {
                    let query = query.unwrap_or_default();
                    mock.reads.lock().unwrap().push(query.clone());
                    let (after, before) =
                        (query_bound(&query, "after"), query_bound(&query, "before"));
                    let activities: Vec<Value> = mock
                        .starts
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|start| {
                            after.is_none_or(|after| **start >= after)
                                && before.is_none_or(|before| **start < before)
                        })
                        .map(|start| session_ride(*start))
                        .collect();
                    Json(json!({
                        "count": activities.len(),
                        "activities": activities,
                        "head_complete": true
                    }))
                }
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), mock)
}

/// Every walk's end the athlete was told.
#[derive(Default)]
struct Told {
    walks: Mutex<Vec<(String, DateTime<Utc>, usize)>>,
}

#[async_trait]
impl BackfillNotifier for Told {
    // A walk is asked by no conversation: nothing here reaches it.
    async fn push_backfill_complete(&self, _: BackfillCompletion<'_>) {}

    async fn push_history_walk_complete(&self, completion: HistoryWalkCompletion<'_>) {
        self.walks.lock().unwrap().push((
            completion.provider.to_owned(),
            completion.oldest,
            completion.activity_count,
        ));
    }

    async fn push_detail_walk_complete(&self, _: DetailWalkCompletion<'_>) {}

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

/// What one test stands up: the server, both stand-ins, the athlete and
/// what they were told.
struct Fixture {
    resources: Arc<ServerContext>,
    session: Arc<MockSession>,
    strava: Arc<MockStrava>,
    told: Arc<Told>,
    user_id: Uuid,
    tenant: TenantId,
}

/// A server reaching the session stand-in over `session_starts` and the
/// Strava stand-in, with a Strava window of `per_window` requests a day, and
/// an athlete holding the usr/pwd session — and the API grant when `grant`.
async fn fixture(session_starts: Vec<i64>, per_window: u32, grant: bool) -> Fixture {
    let (scraper, session) = spawn_session(session_starts).await;
    let (api_base, strava) = mock_strava().await;
    let mut config = common::test_server_config_with_sciotte(&scraper);
    common::set_provider_endpoints(
        &mut config,
        "strava",
        ProviderEndpointOverride {
            api_base_url: Some(api_base),
            ..Default::default()
        },
    );
    common::set_server_app(&mut config, "strava", "walk_client", "walk_secret");
    let base = common::create_test_server_resources_with_config(config)
        .await
        .unwrap();
    let told = Arc::new(Told::default());
    let mut context = (*base).clone();
    let notifier: Arc<dyn BackfillNotifier> = told.clone();
    context.mcp.backfill_notifier = Some(notifier);
    let resources = with_window(&Arc::new(context), per_window);
    let (user_id, tenant) = if grant {
        strava_athlete(&resources).await
    } else {
        let (user_id, _user, tenant) = common::create_test_user_with_plan(
            &resources.agent.database,
            &format!("handoff-{}@example.com", Uuid::new_v4()),
            "starter",
        )
        .await
        .unwrap();
        (user_id, tenant)
    };
    hold_session(&resources, user_id, tenant).await;
    open_window(&resources).await;
    Fixture {
        resources,
        session,
        strava,
        told,
        user_id,
        tenant,
    }
}

/// Seed the athlete's live usr/pwd Strava session and its connection.
async fn hold_session(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId) {
    seed_scrape_session(
        resources,
        user_id,
        tenant,
        oauth_providers::SCIOTTE,
        "handoff-session",
    )
    .await;
    resources
        .common
        .repos
        .provider_connections
        .register_connection(
            user_id,
            tenant,
            oauth_providers::SCIOTTE,
            &ConnectionType::Manual,
            None,
        )
        .await
        .unwrap();
}

impl Fixture {
    fn runtime(&self) -> Arc<dyn ToolRuntime> {
        self.resources.clone()
    }

    /// What the ledger holds for the walk, under the session's key.
    async fn state(&self) -> Option<BackfillJobState> {
        self.resources
            .common
            .repos
            .activity_backfill_jobs
            .backfill_job_state(
                self.user_id,
                self.tenant,
                oauth_providers::SCIOTTE,
                BackfillJobKind::HistoryWalk,
            )
            .await
            .unwrap()
    }

    /// The walk's progress, under the session's key.
    async fn progress(&self) -> JobProgress {
        self.resources
            .common
            .repos
            .activity_backfill_jobs
            .job_progress(
                self.user_id,
                self.tenant,
                oauth_providers::SCIOTTE,
                BackfillJobKind::HistoryWalk,
            )
            .await
            .unwrap()
            .expect("the walk's row is kept")
    }

    /// Run one sweep of the walk, in a fresh Strava window when `fresh`.
    async fn sweep(&self, fresh: bool) {
        if fresh {
            open_window(&self.resources).await;
            paused_retries_due(&self.resources, self.user_id).await;
        }
        age_job_rows(&self.resources, self.user_id).await;
        resume_backfill_jobs(self.runtime()).await.unwrap();
        sleep(Duration::from_millis(50)).await;
    }

    /// Sweep the walk, a fresh window each pass, until its row ends.
    async fn walk_to_the_end(&self) {
        let deadline = Instant::now() + WALK_DEADLINE;
        loop {
            let state = self.state().await;
            if matches!(
                state,
                Some(BackfillJobState::Walked | BackfillJobState::Incomplete)
            ) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the walk did not end within {WALK_DEADLINE:?} (row: {state:?})"
            );
            self.sweep(true).await;
        }
    }

    /// The walks' ends the athlete was told, once every write settled.
    async fn told(&self) -> Vec<(String, DateTime<Utc>, usize)> {
        sleep(Duration::from_millis(200)).await;
        self.told.walks.lock().unwrap().clone()
    }

    /// Activities stored under `key`.
    async fn rows(&self, key: &str) -> i64 {
        rows_under(&self.resources, self.user_id, self.tenant, key).await
    }
}

/// The session lists two rides of the last three years; the API holds three
/// older ones the scraper never listed. The walk reads the session first,
/// then the API strictly below the session's oldest row, 200 a page, down to
/// the account's creation; the progress covers both phases on one row, and
/// the athlete is told once, at the very end, of every activity and the
/// oldest year.
#[tokio::test]
async fn a_session_walk_goes_on_through_the_grant_below_its_oldest_row() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let fx = fixture(vec![now - 3 * DAY, session_oldest], 400, true).await;
    let api_rides = [
        session_oldest - YEAR / 2,
        session_oldest - 3 * YEAR / 2,
        session_oldest - 5 * YEAR / 2,
    ];
    fx.strava.serve(api_rides.to_vec());
    fx.strava.created(now - 6 * YEAR);

    assert!(
        start_history_walk(
            &fx.runtime(),
            fx.user_id,
            fx.tenant,
            oauth_providers::STRAVA,
            WalkTrigger::DepthMissing
        )
        .await,
        "bug: a Strava athlete holding a session is walked"
    );
    fx.walk_to_the_end().await;

    assert_eq!(
        fx.state().await,
        Some(BackfillJobState::Walked),
        "bug: the walk reaches its end through both phases"
    );
    assert_eq!(
        fx.session.read_count(),
        2,
        "bug: the session is read 36 months a step until a step finds nothing, then never again"
    );
    let listings = fx.strava.listings();
    assert!(
        !listings.is_empty(),
        "bug: once the session found nothing more, the grant pages below it"
    );
    assert!(
        listings
            .iter()
            .all(|(_, before)| before.is_some_and(|before| before <= session_oldest)),
        "bug: the API pages strictly below the oldest row the session stored: {listings:?}"
    );
    assert!(
        listings
            .iter()
            .all(|(after, _)| after.is_some_and(|after| after >= now - 6 * YEAR)),
        "bug: the API stops at the account's creation: {listings:?}"
    );
    assert!(
        fx.strava.page_sizes().iter().all(|size| *size == 200),
        "bug: the API phase asks Strava's largest page: {:?}",
        fx.strava.page_sizes()
    );
    assert_eq!(
        fx.strava.profile_count(),
        1,
        "bug: the grant's profile is read once, for the floor"
    );
    assert_eq!(
        fx.rows(oauth_providers::SCIOTTE).await,
        2,
        "bug: the session's rows land under its key"
    );
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        3,
        "bug: the API's rows land under the grant's key"
    );

    let progress = fx.progress().await;
    assert_eq!(
        progress.rows_landed, 5,
        "bug: the progress counts both phases' rows"
    );
    let total = progress
        .units_total
        .expect("the grant's profile names the floor");
    assert!(
        (70..=72).contains(&total),
        "bug: the walk's length runs from now to the account's creation: {progress:?}"
    );
    assert!(
        progress.units_done <= total && progress.units_done >= total - 4,
        "bug: the months done cover both phases without counting the session's empty step twice: {progress:?}"
    );

    let told = fx.told().await;
    assert_eq!(
        told.len(),
        1,
        "bug: the athlete is told once, at the very end: {told:?}"
    );
    let (_, oldest, count) = &told[0];
    assert_eq!(
        *count, 5,
        "bug: the post counts every activity the walk brought in"
    );
    assert_eq!(
        oldest.year(),
        Utc.timestamp_opt(api_rides[2], 0).unwrap().year(),
        "bug: the post names the oldest year either key holds"
    );
}

/// The hand-off is on the row before the API is asked anything: a spent
/// budget pauses the API phase, and a restart (a fresh instance, the sweep
/// rebuilding the job from its row) resumes the API phase below the
/// session, never the session from the top. The athlete is told once.
#[tokio::test]
async fn a_restart_mid_api_phase_resumes_the_grant_not_the_session() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    // Eight requests a window, two of them background: the profile and the
    // first API window, then nothing until the next window.
    let fx = fixture(vec![now - 3 * DAY, session_oldest], 8, true).await;
    fx.strava.serve(vec![
        session_oldest - YEAR / 2,
        session_oldest - 3 * YEAR / 2,
    ]);
    fx.strava.created(now - 5 * YEAR);
    assert!(
        start_history_walk(
            &fx.runtime(),
            fx.user_id,
            fx.tenant,
            oauth_providers::STRAVA,
            WalkTrigger::Connected
        )
        .await,
        "bug: a Strava athlete holding a session is walked"
    );

    // One window: the session phase, then the API phase until the budget
    // refuses it.
    let deadline = Instant::now() + WALK_DEADLINE;
    while fx.strava.listings().is_empty() || fx.progress().await.rows_landed < 3 {
        assert!(Instant::now() < deadline, "the API phase never started");
        fx.sweep(false).await;
    }
    for _ in 0..3 {
        fx.sweep(false).await;
    }
    let session_reads = fx.session.read_count();
    let listed = fx.strava.listings().len();
    assert_eq!(
        listed, 1,
        "bug: a spent budget pauses the API phase after its first window"
    );
    assert_eq!(
        fx.state().await,
        Some(BackfillJobState::Owed),
        "bug: a paused API phase keeps the walk owed"
    );

    // A restart: a fresh instance over the same database, a fresh window.
    let restarted = Fixture {
        resources: with_window(&fx.resources, 400),
        session: Arc::clone(&fx.session),
        strava: Arc::clone(&fx.strava),
        told: Arc::clone(&fx.told),
        user_id: fx.user_id,
        tenant: fx.tenant,
    };
    restarted.walk_to_the_end().await;

    assert_eq!(
        restarted.session.read_count(),
        session_reads,
        "bug: a restart resumes the API phase, never the session from the top"
    );
    assert!(
        restarted.strava.listings().len() > listed,
        "bug: the resumed walk pages the API on from its cursor"
    );
    assert_eq!(
        restarted.rows(oauth_providers::STRAVA).await,
        2,
        "bug: the resumed API phase lands the rest"
    );
    assert_eq!(
        restarted.progress().await.rows_landed,
        4,
        "bug: the progress covers both phases across the restart"
    );
    let told = restarted.told().await;
    assert_eq!(
        told.len(),
        1,
        "bug: the athlete is told once, at the very end: {told:?}"
    );
    assert_eq!(told[0].2, 4, "bug: the post counts both phases");
}

/// A session with no grant beside it ends the walk after its empty step:
/// no API is asked anything, and the athlete is told once. When a grant
/// connects later, the walked session is reopened for the API phase below
/// its oldest row, and that phase's end is told with what it brought in.
#[tokio::test]
async fn a_session_alone_ends_after_its_steps_and_a_later_grant_walks_below_it() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let fx = fixture(vec![now - 3 * DAY, session_oldest], 400, false).await;
    fx.strava.serve(vec![session_oldest - YEAR / 2]);
    fx.strava.created(now - 4 * YEAR);
    let walk = |trigger| {
        let runtime = fx.runtime();
        async move {
            start_history_walk(
                &runtime,
                fx.user_id,
                fx.tenant,
                oauth_providers::STRAVA,
                trigger,
            )
            .await
        }
    };

    assert!(
        walk(WalkTrigger::Connected).await,
        "bug: the session is walked"
    );
    fx.walk_to_the_end().await;
    assert_eq!(
        fx.state().await,
        Some(BackfillJobState::Walked),
        "bug: the session's walk ends"
    );
    assert!(
        fx.strava.listings().is_empty() && fx.strava.profile_count() == 0,
        "bug: with no grant beside the session, no API is asked anything"
    );
    let told = fx.told().await;
    assert_eq!(
        told.len(),
        1,
        "bug: the session's walk is told once: {told:?}"
    );
    assert_eq!(told[0].2, 2, "bug: the post counts the session's rows");
    assert!(
        !walk(WalkTrigger::FirstSync).await,
        "bug: a walked session with no grant beside it is not walked again"
    );

    hold_strava_grant(&fx.resources, fx.user_id, fx.tenant).await;
    assert!(
        walk(WalkTrigger::Connected).await,
        "bug: a grant connected beside a walked session owes the API phase below it"
    );
    assert_eq!(
        fx.state().await,
        Some(BackfillJobState::Owed),
        "bug: the reopened walk is owed"
    );
    fx.walk_to_the_end().await;
    assert_eq!(
        fx.session.read_count(),
        2,
        "bug: the reopened walk never re-reads the session"
    );
    assert!(
        fx.strava
            .listings()
            .iter()
            .all(|(_, before)| before.is_some_and(|before| before <= session_oldest)),
        "bug: the reopened walk pages strictly below the session's oldest row"
    );
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        1,
        "bug: the API's older ride lands"
    );
    let told = fx.told().await;
    assert_eq!(
        told.len(),
        2,
        "bug: the API phase's end is told once more: {told:?}"
    );
    assert_eq!(
        told[1].2, 1,
        "bug: the API phase's post counts what it brought in"
    );
    assert!(
        !walk(WalkTrigger::Connected).await,
        "bug: a walk whose API phase ran is not reopened again"
    );
}

/// A grant connected beside a walked session whose API holds nothing older:
/// the reopened API phase lands nothing, so the athlete, already told how
/// far back their history reaches, is told nothing again.
#[tokio::test]
async fn a_reopened_walk_that_finds_nothing_older_tells_nothing() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let fx = fixture(vec![now - 3 * DAY, session_oldest], 400, false).await;
    fx.strava.serve(vec![now - 3 * DAY]);
    fx.strava.created(now - 4 * YEAR);
    let walk = |trigger| {
        let runtime = fx.runtime();
        async move {
            start_history_walk(
                &runtime,
                fx.user_id,
                fx.tenant,
                oauth_providers::STRAVA,
                trigger,
            )
            .await
        }
    };

    assert!(
        walk(WalkTrigger::Connected).await,
        "bug: the session is walked"
    );
    fx.walk_to_the_end().await;
    assert_eq!(fx.told().await.len(), 1, "the session's walk is told once");

    hold_strava_grant(&fx.resources, fx.user_id, fx.tenant).await;
    assert!(
        walk(WalkTrigger::Connected).await,
        "bug: a grant connected beside a walked session owes the API phase below it"
    );
    fx.walk_to_the_end().await;
    assert_eq!(
        fx.state().await,
        Some(BackfillJobState::Walked),
        "bug: the reopened walk reaches its end"
    );
    assert!(
        !fx.strava.listings().is_empty(),
        "the API phase paged below the session"
    );
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        0,
        "nothing older landed"
    );
    let told = fx.told().await;
    assert_eq!(
        told.len(),
        1,
        "bug: a reopened walk that brought nothing in tells the athlete again: {told:?}"
    );
}

/// The walk handed off to the grant, then the session lapsed. The grant is
/// now the walk's source while its walk still runs under the session's key:
/// a depth ask, the seed or a connect report reaching the grant must see
/// that walk across the pair, and never start a second walk of the same
/// grant from the head. Once that walk paged the grant to its end, none
/// starts either.
#[tokio::test]
async fn a_session_that_lapses_mid_api_phase_leaves_one_walk_of_the_grant() {
    use pierre_core::models::coverage::Pending;
    use pierre_tool_runtime::training_history_compute::history_walk_pending;

    let now = Utc::now().timestamp();
    let fx = fixture(vec![now - 3 * DAY, now - 2 * YEAR], 400, true).await;
    fx.strava.serve(vec![now - 5 * YEAR]);
    fx.strava.created(now - 6 * YEAR);
    let walk = |trigger| {
        let runtime = fx.runtime();
        async move {
            start_history_walk(
                &runtime,
                fx.user_id,
                fx.tenant,
                oauth_providers::STRAVA,
                trigger,
            )
            .await
        }
    };
    let repos = &fx.resources.common.repos;
    let grant_row = || async {
        repos
            .activity_backfill_jobs
            .backfill_job_state(
                fx.user_id,
                fx.tenant,
                oauth_providers::STRAVA,
                BackfillJobKind::HistoryWalk,
            )
            .await
            .unwrap()
    };

    assert!(walk(WalkTrigger::Connected).await, "the session is walked");
    let deadline = Instant::now() + WALK_DEADLINE;
    loop {
        let row = repos
            .activity_backfill_jobs
            .history_walk_row(fx.user_id, fx.tenant, oauth_providers::SCIOTTE)
            .await
            .unwrap()
            .expect("the walk's row is kept");
        if row.walk_backend.as_deref() == Some(oauth_providers::STRAVA) {
            assert_eq!(row.state, BackfillJobState::Owed, "the API phase is owed");
            break;
        }
        assert!(Instant::now() < deadline, "the session never handed off");
        fx.sweep(true).await;
    }
    let listed = fx.strava.listing_count();

    repos
        .provider_connections
        .mark_needs_reauth(
            fx.user_id,
            fx.tenant,
            oauth_providers::SCIOTTE,
            Some("session_expired"),
            Utc::now(),
        )
        .await
        .unwrap();
    for trigger in [
        WalkTrigger::DepthMissing,
        WalkTrigger::Seed,
        WalkTrigger::Connected,
    ] {
        assert!(
            !walk(trigger).await,
            "bug: a {trigger:?} ask starts a second walk of the grant the session's row is paging"
        );
    }
    let pending = history_walk_pending(&fx.runtime(), fx.tenant, fx.user_id)
        .await
        .unwrap();
    assert!(
        matches!(pending, Some(Pending::HistoryWalkRunning { .. })),
        "bug: the depth ask reports the walk paging the grant as running: {pending:?}"
    );
    assert_eq!(
        grant_row().await,
        None,
        "bug: a second walk was filed under the grant's key"
    );
    sleep(Duration::from_millis(200)).await;
    assert_eq!(
        fx.strava.listing_count(),
        listed,
        "bug: the grant was paged by a second walk"
    );

    fx.walk_to_the_end().await;
    assert_eq!(
        fx.state().await,
        Some(BackfillJobState::Walked),
        "bug: the grant's walk did not reach the end of the history"
    );
    assert!(
        !walk(WalkTrigger::DepthMissing).await,
        "bug: a grant the session's walk paged to its end is walked again from the head"
    );
    assert_eq!(grant_row().await, None, "bug: a walk filed under the grant");
}

/// Walk the session through its API phase to the end, then let Strava revoke
/// the grant: the disconnect purges what the grant's phase brought in. The
/// fixture's session stays live.
async fn walked_then_grant_revoked(fx: &Fixture, session_oldest: i64) {
    assert!(
        start_history_walk(
            &fx.runtime(),
            fx.user_id,
            fx.tenant,
            oauth_providers::STRAVA,
            WalkTrigger::Connected
        )
        .await,
        "the session is walked"
    );
    fx.walk_to_the_end().await;
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        3,
        "the API phase landed"
    );
    assert!(
        fx.strava
            .listings()
            .iter()
            .all(|(_, before)| before.is_some_and(|before| before <= session_oldest)),
        "the API phase paged below the session"
    );

    OAuthService::new(fx.resources.data(), Arc::clone(&fx.resources.common.config))
        .disconnect_provider(
            fx.user_id,
            oauth_providers::STRAVA,
            Some(fx.tenant.as_uuid()),
            DisconnectReason::ProviderRevoked,
        )
        .await
        .unwrap();
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        0,
        "the revocation purged what the grant's phase brought in"
    );
    assert_eq!(
        fx.rows(oauth_providers::SCIOTTE).await,
        2,
        "the session's rows stay with the session"
    );
}

/// The walk paged the grant to its end, then the grant was revoked: its
/// purge took the deep history the API phase brought in. The walk no longer
/// claims that phase, so a reconnect beside the live session reopens it,
/// paging the grant again strictly below the session's oldest row.
#[tokio::test]
async fn a_purged_grant_reconnected_beside_its_session_is_walked_again_below_it() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let fx = fixture(vec![now - 3 * DAY, session_oldest], 400, true).await;
    fx.strava.serve(vec![
        session_oldest - YEAR / 2,
        session_oldest - 3 * YEAR / 2,
        session_oldest - 5 * YEAR / 2,
    ]);
    fx.strava.created(now - 6 * YEAR);
    walked_then_grant_revoked(&fx, session_oldest).await;
    let listed = fx.strava.listing_count();

    hold_strava_grant(&fx.resources, fx.user_id, fx.tenant).await;
    assert!(
        start_history_walk(
            &fx.runtime(),
            fx.user_id,
            fx.tenant,
            oauth_providers::STRAVA,
            WalkTrigger::Connected
        )
        .await,
        "bug: a grant whose purge took the API phase's history is never walked again"
    );
    fx.walk_to_the_end().await;
    let again = &fx.strava.listings()[listed..];
    assert!(
        !again.is_empty()
            && again
                .iter()
                .all(|(_, before)| before.is_some_and(|before| before <= session_oldest)),
        "bug: the reconnected grant is paged again strictly below the session: {again:?}"
    );
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        3,
        "bug: the purged deep history is read back"
    );
    assert_eq!(
        fx.session.read_count(),
        2,
        "bug: the session's phase is not walked again"
    );
}

/// The same purge, but the session has lapsed by the time the grant comes
/// back: the grant is now the walk's source, and a walk of it starts under
/// its own key from the head, reading the whole history the grant lists —
/// the recent rides the session held and the deep ones the purge took.
#[tokio::test]
async fn a_purged_grant_reconnected_after_its_session_lapsed_is_walked_again() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let fx = fixture(vec![now - 3 * DAY, session_oldest], 400, true).await;
    fx.strava.serve(vec![
        session_oldest - YEAR / 2,
        session_oldest - 3 * YEAR / 2,
        session_oldest - 5 * YEAR / 2,
    ]);
    fx.strava.created(now - 6 * YEAR);
    walked_then_grant_revoked(&fx, session_oldest).await;
    // A grant lists every ride, the session's recent ones too.
    fx.strava.serve(vec![
        now - 3 * DAY,
        session_oldest,
        session_oldest - YEAR / 2,
        session_oldest - 3 * YEAR / 2,
        session_oldest - 5 * YEAR / 2,
    ]);
    let repos = &fx.resources.common.repos;
    repos
        .provider_connections
        .mark_needs_reauth(
            fx.user_id,
            fx.tenant,
            oauth_providers::SCIOTTE,
            Some("session_expired"),
            Utc::now(),
        )
        .await
        .unwrap();

    hold_strava_grant(&fx.resources, fx.user_id, fx.tenant).await;
    assert!(
        start_history_walk(
            &fx.runtime(),
            fx.user_id,
            fx.tenant,
            oauth_providers::STRAVA,
            WalkTrigger::Connected
        )
        .await,
        "bug: a grant whose purge took the API phase's history is never walked again"
    );
    let deadline = Instant::now() + WALK_DEADLINE;
    loop {
        let row = repos
            .activity_backfill_jobs
            .backfill_job_state(
                fx.user_id,
                fx.tenant,
                oauth_providers::STRAVA,
                BackfillJobKind::HistoryWalk,
            )
            .await
            .unwrap();
        if row == Some(BackfillJobState::Walked) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the grant's walk did not end (row: {row:?})"
        );
        fx.sweep(true).await;
    }
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        5,
        "bug: the grant's walk reads its whole history back, the purged deep rides included"
    );
}

/// Walk the session to its end and the grant's first year below it, then
/// catch the grant's second year mid-request: its listing is held until the
/// returned gate is notified. The session listed `session_oldest`; the grant
/// serves one ride in each of the three years below it.
async fn hold_the_second_api_year(fx: &Fixture, session_oldest: i64) -> Arc<Notify> {
    fx.strava.serve(vec![
        session_oldest - YEAR / 2,
        session_oldest - 3 * YEAR / 2,
        session_oldest - 5 * YEAR / 2,
    ]);
    fx.strava.created(Utc::now().timestamp() - 6 * YEAR);
    assert!(
        start_history_walk(
            &fx.runtime(),
            fx.user_id,
            fx.tenant,
            oauth_providers::STRAVA,
            WalkTrigger::Connected
        )
        .await,
        "the walk of the session starts"
    );
    let deadline = Instant::now() + WALK_DEADLINE;
    while !(fx.rows(oauth_providers::STRAVA).await == 1 && fx.idle().await) {
        assert!(
            Instant::now() < deadline,
            "the grant's first year never landed"
        );
        fx.sweep(true).await;
    }
    let gate = fx.strava.hold_next_listing();
    fx.sweep(true).await;
    fx.strava.listing_held().await;
    gate
}

impl Fixture {
    /// Whether the walk under the session's key is owed with no step holding it.
    async fn idle(&self) -> bool {
        self.resources
            .common
            .repos
            .activity_backfill_jobs
            .backfill_job_lease_state(
                self.user_id,
                self.tenant,
                oauth_providers::SCIOTTE,
                BackfillJobKind::HistoryWalk,
                Utc::now().timestamp_millis(),
            )
            .await
            .unwrap()
            == Some(JobLeaseState::Idle)
    }

    /// Revoke the grant: its purge takes what the API phase brought in.
    async fn revoke_grant(&self) {
        OAuthService::new(
            self.resources.data(),
            Arc::clone(&self.resources.common.config),
        )
        .disconnect_provider(
            self.user_id,
            oauth_providers::STRAVA,
            Some(self.tenant.as_uuid()),
            DisconnectReason::ProviderRevoked,
        )
        .await
        .unwrap();
        assert_eq!(
            self.rows(oauth_providers::STRAVA).await,
            0,
            "the purge took the grant's rows"
        );
    }

    /// Connect the grant again and ask for its walk, as a connect does.
    async fn reconnect(&self) -> bool {
        hold_strava_grant(&self.resources, self.user_id, self.tenant).await;
        start_history_walk(
            &self.runtime(),
            self.user_id,
            self.tenant,
            oauth_providers::STRAVA,
            WalkTrigger::Connected,
        )
        .await
    }
}

/// The grant is revoked while a step of the walk's API phase is reading its
/// second year. The step finds its phase purged before it writes, so none of
/// that year lands under the purged grant's key. A reconnect afterwards
/// reopens the walk below the session's own oldest row, so the purged
/// history is read again.
#[tokio::test]
async fn a_purge_mid_step_then_a_reconnect_reads_the_purged_range_again() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let fx = fixture(vec![now - 3 * DAY, session_oldest], 400, true).await;
    let gate = hold_the_second_api_year(&fx, session_oldest).await;
    fx.revoke_grant().await;
    gate.notify_one();
    fx.step_stopped().await;
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        0,
        "bug: a step whose grant was purged mid-request wrote the purged grant's rows back"
    );

    let deadline = Instant::now() + WALK_DEADLINE;
    let listed = fx.strava.listing_count();
    // A reconnect the running step refused is asked again by that step once
    // it stops, which reopens the walk itself.
    while !(fx.reconnect().await || fx.state().await == Some(BackfillJobState::Owed)) {
        assert!(
            Instant::now() < deadline,
            "bug: the step of a purged phase never gave its lease up, so a reconnect cannot reopen the walk"
        );
        sleep(Duration::from_millis(100)).await;
    }
    let coverage = fx
        .resources
        .common
        .repos
        .activity_cache
        .get_backfill_coverage(fx.user_id, &fx.tenant, oauth_providers::STRAVA)
        .await
        .unwrap();
    assert!(
        coverage.is_none(),
        "bug: a step whose phase the purge reset claimed coverage for the grant: {coverage:?}"
    );
    fx.walk_to_the_end().await;
    let again = fx.strava.listings()[listed..].to_vec();
    assert_eq!(
        again.first().and_then(|(_, before)| *before),
        Some(session_oldest),
        "bug: the reopened walk starts below a stray page, not the session's oldest row: {again:?}"
    );
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        3,
        "bug: the purged year above the stray page is never read again"
    );
}

/// The grant is revoked and connected again while a step of the walk's API
/// phase still runs. The reconnect cannot reopen the walk under the running
/// step, and no sweep pages the grant beside it; once the step finds its
/// phase reset it gives the lease up and asks for the walk again, which the
/// live grant reopens below the session, reading the purged history back.
#[tokio::test]
async fn a_reconnect_during_a_purged_step_is_walked_once_the_step_stops() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let fx = fixture(vec![now - 3 * DAY, session_oldest], 400, true).await;
    let gate = hold_the_second_api_year(&fx, session_oldest).await;
    fx.revoke_grant().await;
    assert!(
        !fx.reconnect().await,
        "bug: a reconnect reopened the walk under a step still running"
    );
    let listed = fx.strava.listing_count();
    fx.sweep(true).await;
    fx.sweep(true).await;
    assert_eq!(
        fx.strava.listing_count(),
        listed,
        "bug: a second step paged the grant beside the running one"
    );

    gate.notify_one();
    let deadline = Instant::now() + WALK_DEADLINE;
    while fx.state().await != Some(BackfillJobState::Owed) {
        assert!(
            Instant::now() < deadline,
            "bug: the step that found its phase reset never asked for the walk again"
        );
        sleep(Duration::from_millis(50)).await;
    }
    fx.walk_to_the_end().await;
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        3,
        "bug: the reconnected grant's purged history is not read back"
    );
    assert_eq!(
        fx.state().await,
        Some(BackfillJobState::Walked),
        "bug: the reconnected grant's walk did not reach the end of the history"
    );
}

/// The same reconnect during the purged step, but the step's request then
/// fails, as a revoked grant's does: it writes nothing, yet it still hears
/// its phase was reset as it gives its lease back, and asks for the walk
/// again, which the reconnected grant reopens at once rather than a lease
/// later.
#[tokio::test]
async fn a_reconnect_during_a_purged_step_that_fails_is_walked_once_it_stops() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let fx = fixture(vec![now - 3 * DAY, session_oldest], 400, true).await;
    let gate = hold_the_second_api_year(&fx, session_oldest).await;
    fx.strava.fail_the_held_listing();
    fx.revoke_grant().await;
    assert!(
        !fx.reconnect().await,
        "bug: a reconnect reopened the walk under a step still running"
    );

    gate.notify_one();
    let deadline = Instant::now() + WALK_DEADLINE;
    while fx.state().await != Some(BackfillJobState::Owed) {
        assert!(
            Instant::now() < deadline,
            "bug: a failed step of a purged phase kept its lease and never asked for the walk again"
        );
        sleep(Duration::from_millis(50)).await;
    }
    fx.walk_to_the_end().await;
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        3,
        "bug: the reconnected grant's purged history is not read back"
    );
}

/// An old live read left one deep page under the grant's key before the walk
/// ever ran: a ride well below the session's reach. When the session's phase
/// ends, the API phase starts below the oldest row the session itself
/// stored, never below that stray page, so the years between the session's
/// oldest row and the stray ride are paged too.
#[tokio::test]
async fn the_api_phase_starts_below_the_sessions_own_oldest_row_not_a_stray_grant_page() {
    use pierre_core::models::{ActivityBuilder, SportType};

    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let fx = fixture(vec![now - 3 * DAY, session_oldest], 400, true).await;
    let stray = session_oldest - 5 * YEAR / 2;
    fx.strava.serve(vec![
        session_oldest - YEAR / 2,
        session_oldest - 3 * YEAR / 2,
        stray,
    ]);
    fx.strava.created(now - 6 * YEAR);
    let stray_ride = ActivityBuilder::new(
        stray.to_string(),
        "Morning ride".to_owned(),
        SportType::Ride,
        Utc.timestamp_opt(stray, 0).unwrap(),
        3_600,
        oauth_providers::STRAVA.to_owned(),
    )
    .build();
    fx.resources
        .common
        .repos
        .activity_cache
        .upsert_activities(
            fx.user_id,
            &fx.tenant,
            oauth_providers::STRAVA,
            &[stray_ride],
        )
        .await
        .unwrap();

    assert!(
        start_history_walk(
            &fx.runtime(),
            fx.user_id,
            fx.tenant,
            oauth_providers::STRAVA,
            WalkTrigger::Connected
        )
        .await,
        "the walk of the session starts"
    );
    fx.walk_to_the_end().await;

    let listings = fx.strava.listings();
    assert_eq!(
        listings.first().and_then(|(_, before)| *before),
        Some(session_oldest),
        "bug: the API phase starts below a stray grant page, not the session's oldest row: {listings:?}"
    );
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        3,
        "bug: the years between the session's oldest row and the stray page are never paged"
    );
}

impl Fixture {
    /// Sweep the walk until its session handed off to the grant and no step
    /// holds the row: the API phase is owed, its next step not yet taken.
    async fn reach_the_api_phase(&self) {
        let deadline = Instant::now() + WALK_DEADLINE;
        loop {
            let row = self
                .resources
                .common
                .repos
                .activity_backfill_jobs
                .history_walk_row(self.user_id, self.tenant, oauth_providers::SCIOTTE)
                .await
                .unwrap()
                .expect("the walk's row is kept");
            if row.walk_backend.as_deref() == Some(oauth_providers::STRAVA) && self.idle().await {
                return;
            }
            assert!(Instant::now() < deadline, "the session never handed off");
            self.sweep(true).await;
        }
    }

    /// Claim the walk's row the way another instance's sweep does, leased
    /// for a whole lease, and never run it: the instance dies holding it.
    async fn claim_as_a_runner_that_dies(&self) {
        age_job_rows(&self.resources, self.user_id).await;
        let claimed = self
            .resources
            .common
            .repos
            .activity_backfill_jobs
            .claim_stale_backfill_jobs(Self::claim_at(Utc::now().timestamp_millis()))
            .await
            .unwrap();
        assert_eq!(claimed.len(), 1, "the dead runner holds the walk's row");
    }

    /// The resume sweep's claim at `now_ms`, over every row however fresh.
    fn claim_at(now_ms: i64) -> BackfillJobClaim {
        BackfillJobClaim {
            now_ms,
            queued_older_than_ms: 0,
            lease_ms: i64::try_from(BACKFILL_JOB_LEASE.as_millis()).unwrap(),
            max_attempts: MAX_BACKFILL_ATTEMPTS,
            limit: 10,
        }
    }

    /// Let every lease the athlete's rows hold run out, as a dead runner's
    /// does once its lease has passed.
    async fn lapse_leases(&self) {
        let sql = "UPDATE activity_backfill_jobs SET leased_until_ms = 0 WHERE user_id = $1";
        match self.resources.agent.database.backend() {
            DatabaseBackend::SQLite(sqlite) => {
                sqlx::query(sql)
                    .bind(self.user_id.to_string())
                    .execute(sqlite.pool())
                    .await
                    .unwrap();
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(pg) => {
                sqlx::query(sql)
                    .bind(self.user_id.to_string())
                    .execute(pg.pool())
                    .await
                    .unwrap();
            }
        }
    }

    /// The usr/pwd session expires: the grant becomes the walk's source.
    async fn lapse_session(&self) {
        self.resources
            .common
            .repos
            .provider_connections
            .mark_needs_reauth(
                self.user_id,
                self.tenant,
                oauth_providers::SCIOTTE,
                Some("session_expired"),
                Utc::now(),
            )
            .await
            .unwrap();
    }

    /// What the ledger holds under the grant's own key.
    async fn grant_row(&self) -> Option<BackfillJobState> {
        self.resources
            .common
            .repos
            .activity_backfill_jobs
            .backfill_job_state(
                self.user_id,
                self.tenant,
                oauth_providers::STRAVA,
                BackfillJobKind::HistoryWalk,
            )
            .await
            .unwrap()
    }
}

/// The instance running a step of the walk's API phase dies just as the
/// grant is revoked, and the grant connects again at once. The reconnect
/// cannot reopen the walk under the dead runner's lease, and no step of the
/// dead runner ever asks for the walk again. Once that lease has run out,
/// the resume sweep re-walks the grant below the session's own oldest row,
/// reading the purged history back, without any further ask.
#[tokio::test]
async fn a_purge_under_a_dead_runner_is_walked_again_once_its_lease_lapses() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let fx = fixture(vec![now - 3 * DAY, session_oldest], 400, true).await;
    fx.strava.serve(vec![
        session_oldest - YEAR / 2,
        session_oldest - 3 * YEAR / 2,
        session_oldest - 5 * YEAR / 2,
    ]);
    fx.strava.created(now - 6 * YEAR);
    assert!(
        start_history_walk(
            &fx.runtime(),
            fx.user_id,
            fx.tenant,
            oauth_providers::STRAVA,
            WalkTrigger::Connected
        )
        .await,
        "the walk of the session starts"
    );
    fx.reach_the_api_phase().await;
    fx.claim_as_a_runner_that_dies().await;
    fx.revoke_grant().await;
    assert!(
        !fx.reconnect().await,
        "bug: a reconnect reopened the walk under a lease a runner still holds"
    );
    let listed = fx.strava.listing_count();
    fx.sweep(true).await;
    assert_eq!(
        fx.strava.listing_count(),
        listed,
        "bug: the grant was paged while the dead runner's lease still held"
    );

    fx.lapse_leases().await;
    fx.walk_to_the_end().await;
    let again = fx.strava.listings()[listed..].to_vec();
    assert_eq!(
        again.first().and_then(|(_, before)| *before),
        Some(session_oldest),
        "bug: once the dead runner's lease lapsed, the purged walk is not re-walked below the session's oldest row: {again:?}"
    );
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        3,
        "bug: the purged history is never read back"
    );
    assert_eq!(
        fx.state().await,
        Some(BackfillJobState::Walked),
        "bug: the walk read again after the purge did not reach the end of the history"
    );
}

/// The same dead runner, but the session lapsed before the grant came back:
/// the grant is now the walk's source. Another instance handling the
/// reconnect must still see the purged walk of the pair under the runner's
/// lease and start no walk of the grant from the head beside it — the only
/// thing holding it back across instances is the ledger, since the dead
/// runner's in-flight key was never this process's. Once the lease lapses,
/// the sweep pages the grant below the session's oldest row, under the
/// session's key.
#[tokio::test]
async fn a_reconnect_after_the_session_lapsed_never_pages_the_grant_beside_a_purged_step() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let fx = fixture(vec![now - 3 * DAY, session_oldest], 400, true).await;
    fx.strava.serve(vec![
        session_oldest - YEAR / 2,
        session_oldest - 3 * YEAR / 2,
        session_oldest - 5 * YEAR / 2,
    ]);
    fx.strava.created(now - 6 * YEAR);
    assert!(
        start_history_walk(
            &fx.runtime(),
            fx.user_id,
            fx.tenant,
            oauth_providers::STRAVA,
            WalkTrigger::Connected
        )
        .await,
        "the walk of the session starts"
    );
    fx.reach_the_api_phase().await;
    fx.claim_as_a_runner_that_dies().await;
    fx.revoke_grant().await;
    fx.lapse_session().await;
    let listed = fx.strava.listing_count();

    assert!(
        !fx.reconnect().await,
        "bug: a reconnect started a walk of the grant beside the purged step still holding its lease"
    );
    for trigger in [WalkTrigger::DepthMissing, WalkTrigger::Seed] {
        assert!(
            !start_history_walk(
                &fx.runtime(),
                fx.user_id,
                fx.tenant,
                oauth_providers::STRAVA,
                trigger
            )
            .await,
            "bug: a {trigger:?} ask started a walk of the grant beside the purged step"
        );
    }
    assert_eq!(
        fx.grant_row().await,
        None,
        "bug: a walk was filed under the grant's key beside the purged step"
    );
    sleep(Duration::from_millis(200)).await;
    assert_eq!(
        fx.strava.listing_count(),
        listed,
        "bug: the grant was paged from the head beside the purged step"
    );

    fx.lapse_leases().await;
    fx.walk_to_the_end().await;
    let again = fx.strava.listings()[listed..].to_vec();
    assert_eq!(
        again.first().and_then(|(_, before)| *before),
        Some(session_oldest),
        "bug: the purged walk is not re-walked below the session's oldest row: {again:?}"
    );
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        3,
        "bug: the grant's rows the walk read again are not all in the store"
    );
    assert_eq!(
        fx.grant_row().await,
        None,
        "bug: the grant was walked under its own key"
    );
}

/// Fail the API phase's next step: its listing is answered with an error.
/// Returns the instants just before the step ran and just after it settled.
async fn fail_the_next_api_step(fx: &Fixture) -> (i64, i64) {
    fx.reach_the_api_phase().await;
    let gate = fx.strava.hold_next_listing();
    fx.strava.fail_the_held_listing();
    let before = Utc::now().timestamp_millis();
    fx.sweep(true).await;
    fx.strava.listing_held().await;
    gate.notify_one();
    let deadline = Instant::now() + WALK_DEADLINE;
    while !fx.idle().await {
        assert!(Instant::now() < deadline, "the failed step never settled");
        sleep(Duration::from_millis(50)).await;
    }
    (before, Utc::now().timestamp_millis())
}

/// A failed walk step gives its row back with its retry a whole lease
/// later: the sweep takes it neither a minute after the failure nor just
/// before that lease has passed, and does once it has.
#[tokio::test]
async fn a_failed_walk_steps_retry_is_due_a_whole_lease_later() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let fx = fixture(vec![now - 3 * DAY, session_oldest], 400, true).await;
    fx.strava.serve(vec![session_oldest - YEAR / 2]);
    fx.strava.created(now - 6 * YEAR);
    assert!(
        start_history_walk(
            &fx.runtime(),
            fx.user_id,
            fx.tenant,
            oauth_providers::STRAVA,
            WalkTrigger::Connected
        )
        .await,
        "the walk of the session starts"
    );
    let (before, after) = fail_the_next_api_step(&fx).await;
    assert_eq!(
        fx.state().await,
        Some(BackfillJobState::Owed),
        "a failed step keeps the walk owed"
    );
    let lease = i64::try_from(BACKFILL_JOB_LEASE.as_millis()).unwrap();
    let ledger = &fx.resources.common.repos.activity_backfill_jobs;
    for now_ms in [after + 60_000, before + lease - 2_000] {
        let swept = ledger
            .claim_stale_backfill_jobs(Fixture::claim_at(now_ms))
            .await
            .unwrap();
        assert!(
            swept.is_empty(),
            "bug: a failed walk step is retried {}s after it failed, before a whole lease: {swept:?}",
            (now_ms - after) / 1_000
        );
    }
    let swept = ledger
        .claim_stale_backfill_jobs(Fixture::claim_at(after + lease + 2_000))
        .await
        .unwrap();
    assert_eq!(
        swept.len(),
        1,
        "bug: the failed walk step is not due once a whole lease has passed"
    );
}

/// A step of the API phase failed, so its retry is due a lease later; then
/// the grant is revoked and connected again. The reopened walk is a new
/// phase, owed nothing of the failed step's backoff: the sweep takes it at
/// once.
#[tokio::test]
async fn a_walk_reopened_after_a_failed_step_is_claimable_at_once() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let fx = fixture(vec![now - 3 * DAY, session_oldest], 400, true).await;
    fx.strava.serve(vec![session_oldest - YEAR / 2]);
    fx.strava.created(now - 6 * YEAR);
    assert!(
        start_history_walk(
            &fx.runtime(),
            fx.user_id,
            fx.tenant,
            oauth_providers::STRAVA,
            WalkTrigger::Connected
        )
        .await,
        "the walk of the session starts"
    );
    fail_the_next_api_step(&fx).await;
    fx.revoke_grant().await;
    assert!(
        fx.reconnect().await,
        "a reconnect with no step holding the walk reopens it"
    );
    let swept = fx
        .resources
        .common
        .repos
        .activity_backfill_jobs
        .claim_stale_backfill_jobs(Fixture::claim_at(Utc::now().timestamp_millis() + 1_000))
        .await
        .unwrap();
    assert_eq!(
        swept.len(),
        1,
        "bug: a reopened walk waits out the backoff of a step from before the purge"
    );
    assert_eq!(
        swept[0].walk_backend.as_deref(),
        Some(oauth_providers::STRAVA),
        "bug: the reopened walk is not walked through the grant"
    );
}

impl Fixture {
    /// Wait until no step holds the walk's row under the session's key: the
    /// step that ran stopped and gave its lease up, and whatever it asked
    /// for after had time to run.
    async fn step_stopped(&self) {
        let sql = "SELECT leased_until_ms FROM activity_backfill_jobs \
                   WHERE user_id = $1 AND provider = $2 AND kind = $3";
        let deadline = Instant::now() + WALK_DEADLINE;
        loop {
            let leased_until: i64 = match self.resources.agent.database.backend() {
                DatabaseBackend::SQLite(sqlite) => sqlx::query_scalar(sql)
                    .bind(self.user_id.to_string())
                    .bind(oauth_providers::SCIOTTE)
                    .bind(BackfillJobKind::HistoryWalk.as_str())
                    .fetch_one(sqlite.pool())
                    .await
                    .unwrap(),
                #[cfg(feature = "postgresql")]
                DatabaseBackend::PostgreSQL(pg) => sqlx::query_scalar(sql)
                    .bind(self.user_id.to_string())
                    .bind(oauth_providers::SCIOTTE)
                    .bind(BackfillJobKind::HistoryWalk.as_str())
                    .fetch_one(pg.pool())
                    .await
                    .unwrap(),
            };
            if leased_until <= Utc::now().timestamp_millis() {
                sleep(Duration::from_millis(300)).await;
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the held step never gave its lease up"
            );
            sleep(Duration::from_millis(50)).await;
        }
    }

    /// The operator terminates Strava: every athlete's Strava data goes,
    /// from the tables and the activity store. The grants stay: the purge
    /// deletes data, never tokens.
    async fn terminate_strava(&self) {
        self.resources
            .common
            .provider_purge
            .purge_provider_data(oauth_providers::STRAVA)
            .await
            .unwrap();
        assert_eq!(
            self.rows(oauth_providers::STRAVA).await,
            0,
            "the termination took the grant's rows"
        );
    }

    /// The walk under the session's key: walked alone, with no API phase
    /// owed, and no step of the grant beside it.
    async fn assert_walked_alone(&self, listed: usize, what: &str) {
        for _ in 0..2 {
            self.lapse_leases().await;
            self.sweep(true).await;
        }
        let row = self
            .resources
            .common
            .repos
            .activity_backfill_jobs
            .history_walk_row(self.user_id, self.tenant, oauth_providers::SCIOTTE)
            .await
            .unwrap()
            .expect("the walk's row is kept");
        assert_eq!(
            (row.state, row.walk_backend),
            (BackfillJobState::Walked, None),
            "bug: {what} reopened the walk through the terminated provider"
        );
        assert_eq!(
            self.grant_row().await,
            None,
            "bug: {what} started a walk of the terminated grant from the head"
        );
        assert_eq!(
            self.strava.listing_count(),
            listed,
            "bug: {what} paged the terminated provider again"
        );
    }
}

/// The operator terminates Strava while a step of the walk's API phase is
/// reading its second year. The grant stays live — the purge deletes data,
/// never tokens — yet the step that finds its phase gone asks nothing of it:
/// the walk is left as its session walked alone, never reopened through the
/// terminated provider.
#[tokio::test]
async fn a_termination_under_a_running_step_reopens_nothing() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let fx = fixture(vec![now - 3 * DAY, session_oldest], 400, true).await;
    let gate = hold_the_second_api_year(&fx, session_oldest).await;
    fx.terminate_strava().await;
    let listed = fx.strava.listing_count();
    gate.notify_one();
    fx.step_stopped().await;
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        0,
        "bug: the step wrote the terminated provider's rows back"
    );
    fx.assert_walked_alone(listed, "the step a termination stopped")
        .await;
}

/// The same termination under a running step, but the session lapsed
/// meanwhile, so the still-live grant is the pair's source: the step that
/// stops starts no walk of the terminated grant from the head.
#[tokio::test]
async fn a_termination_under_a_running_step_starts_no_walk_of_the_grant() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let fx = fixture(vec![now - 3 * DAY, session_oldest], 400, true).await;
    let gate = hold_the_second_api_year(&fx, session_oldest).await;
    fx.terminate_strava().await;
    fx.lapse_session().await;
    let listed = fx.strava.listing_count();
    gate.notify_one();
    fx.step_stopped().await;
    fx.assert_walked_alone(listed, "the step a termination stopped")
        .await;
}

/// A grant's purge left the walk owed a reopen under a runner that died, the
/// grant connected again, and the operator then terminates Strava before
/// the dead runner's lease lapsed. The termination settles the walk as its
/// session walked alone, so once the lease lapses the resume sweep reopens
/// nothing through the terminated provider's live grant.
#[tokio::test]
async fn a_termination_settles_a_purged_walk_the_sweep_would_have_reopened() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let fx = fixture(vec![now - 3 * DAY, session_oldest], 400, true).await;
    fx.strava.serve(vec![
        session_oldest - YEAR / 2,
        session_oldest - 3 * YEAR / 2,
        session_oldest - 5 * YEAR / 2,
    ]);
    fx.strava.created(now - 6 * YEAR);
    assert!(
        start_history_walk(
            &fx.runtime(),
            fx.user_id,
            fx.tenant,
            oauth_providers::STRAVA,
            WalkTrigger::Connected
        )
        .await,
        "the walk of the session starts"
    );
    fx.reach_the_api_phase().await;
    fx.claim_as_a_runner_that_dies().await;
    fx.revoke_grant().await;
    assert!(
        !fx.reconnect().await,
        "a reconnect cannot reopen the walk under the dead runner's lease"
    );
    fx.terminate_strava().await;
    let listed = fx.strava.listing_count();
    fx.assert_walked_alone(listed, "the resume sweep").await;
}

/// The live activity store the server writes through, with one write held:
/// once armed, the next write of `provider` waits for the test before it
/// lands, so a purge can commit between a step's check of its row and its
/// write.
struct HeldWrite {
    inner: Arc<dyn LiveActivityStore>,
    armed: Mutex<Option<Hold>>,
}

/// One armed hold: the provider whose next write waits, the gate that lets
/// it land, the signals that it reached the store and that it landed, and,
/// when set, the gate that lets the write return to its step once landed.
#[derive(Clone)]
struct Hold {
    provider: String,
    gate: Arc<Notify>,
    reached: Arc<Notify>,
    landed: Arc<Notify>,
    resume: Option<Arc<Notify>>,
}

impl HeldWrite {
    /// Hold the next write of `provider`.
    fn hold_next_write(&self, provider: &str) -> Hold {
        let hold = Hold {
            provider: provider.to_owned(),
            gate: Arc::new(Notify::new()),
            reached: Arc::new(Notify::new()),
            landed: Arc::new(Notify::new()),
            resume: None,
        };
        *self.armed.lock().unwrap() = Some(hold.clone());
        hold
    }

    /// Let the next write of `provider` land at once, then hold it before
    /// it returns to its step, until the returned hold's `resume` is
    /// notified.
    fn pause_after_next_write(&self, provider: &str) -> Hold {
        let hold = Hold {
            resume: Some(Arc::new(Notify::new())),
            ..self.hold_next_write(provider)
        };
        hold.gate.notify_one();
        *self.armed.lock().unwrap() = Some(hold.clone());
        hold
    }

    /// Hold the next write of `provider` until the returned hold's `gate` is
    /// notified, then, once it landed, hold it again before it returns to
    /// its step until `resume` is notified.
    fn hold_and_pause_next_write(&self, provider: &str) -> Hold {
        let hold = Hold {
            resume: Some(Arc::new(Notify::new())),
            ..self.hold_next_write(provider)
        };
        *self.armed.lock().unwrap() = Some(hold.clone());
        hold
    }
}

#[async_trait]
impl LiveActivityStore for HeldWrite {
    async fn ingest(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activities: &[Activity],
        synced_at: DateTime<Utc>,
    ) -> AppResult<LiveIngest> {
        let held = {
            let mut armed = self.armed.lock().unwrap();
            if armed.as_ref().is_some_and(|hold| hold.provider == provider) {
                armed.take()
            } else {
                None
            }
        };
        if let Some(hold) = &held {
            hold.reached.notify_one();
            hold.gate.notified().await;
        }
        let ingested = self
            .inner
            .ingest(user_id, tenant_id, provider, activities, synced_at)
            .await;
        if let Some(hold) = held {
            hold.landed.notify_one();
            if let Some(resume) = hold.resume {
                resume.notified().await;
            }
        }
        ingested
    }

    async fn ingest_received(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activity: &Activity,
        received_at: DateTime<Utc>,
    ) -> AppResult<bool> {
        self.inner
            .ingest_received(user_id, tenant_id, provider, activity, received_at)
            .await
    }

    async fn amend(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activity: &Activity,
    ) -> AppResult<bool> {
        self.inner
            .amend(user_id, tenant_id, provider, activity)
            .await
    }

    async fn erase(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activity_id: &str,
    ) -> AppResult<bool> {
        self.inner
            .erase(user_id, tenant_id, provider, activity_id)
            .await
    }

    async fn discard(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activity_id: &str,
    ) -> AppResult<bool> {
        self.inner
            .discard(user_id, tenant_id, provider, activity_id)
            .await
    }

    async fn discard_between(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<u64> {
        self.inner
            .discard_between(user_id, tenant_id, provider, start, end)
            .await
    }

    async fn held(&self, user_id: Uuid, tenant_id: TenantId, provider: &str) -> AppResult<u64> {
        self.inner.held(user_id, tenant_id, provider).await
    }

    async fn held_among(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activities: &[Activity],
    ) -> AppResult<HashSet<String>> {
        self.inner
            .held_among(user_id, tenant_id, provider, activities)
            .await
    }

    async fn recorded_among(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
        activities: &[Activity],
    ) -> AppResult<HashSet<String>> {
        self.inner
            .recorded_among(user_id, tenant_id, provider, activities)
            .await
    }

    async fn delete_source(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<()> {
        self.inner.delete_source(user_id, tenant_id, provider).await
    }

    fn keeps(&self, provider: &str) -> bool {
        self.inner.keeps(provider)
    }

    async fn delete_source_everywhere(&self, provider: &str) -> AppResult<u64> {
        self.inner.delete_source_everywhere(provider).await
    }

    async fn delete_user(&self, user_id: Uuid) -> AppResult<u64> {
        self.inner.delete_user(user_id).await
    }
}

impl Fixture {
    /// Write every activity through a store whose writes the test can hold
    /// ([`HeldWrite`]), before any walk runs.
    fn hold_writes(&mut self) -> Arc<HeldWrite> {
        let mut context = (*self.resources).clone();
        let held = Arc::new(HeldWrite {
            inner: Arc::clone(&context.common.live_activities),
            armed: Mutex::new(None),
        });
        let store: Arc<dyn LiveActivityStore> = held.clone();
        context.common.live_activities = store;
        self.resources = Arc::new(context);
        held
    }
}

/// The grant is revoked after a step of the walk's API phase found its row
/// still owed and before its write landed: the write lands after the purge.
/// The step sees its phase purged once it wrote, and takes back exactly the
/// rows it wrote, so nothing of the purged grant is left in the store.
#[tokio::test]
async fn a_purge_landing_while_a_step_writes_leaves_none_of_its_rows() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let mut fx = fixture(vec![now - 3 * DAY, session_oldest], 400, true).await;
    let store = fx.hold_writes();
    let listing = hold_the_second_api_year(&fx, session_oldest).await;
    let write = store.hold_next_write(oauth_providers::STRAVA);
    listing.notify_one();
    write.reached.notified().await;
    fx.revoke_grant().await;
    write.gate.notify_one();
    fx.step_stopped().await;
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        0,
        "bug: a step's write that landed after its grant's purge left the purged grant's rows in the store"
    );
    assert_eq!(
        fx.rows(oauth_providers::SCIOTTE).await,
        2,
        "the session's rows stay with the session"
    );
}

/// The athlete drops both connections while a step of the walk's API phase
/// writes the grant's page: the session's purge removes the walk's row, so
/// the grant's purge finds no row to mark, and the step's write lands after
/// both. With its row gone the step reads the grant's purge from the
/// attestation it wrote, and takes its page back: nothing of the purged
/// grant is left in the store.
#[tokio::test]
async fn a_step_whose_row_both_purges_took_leaves_none_of_its_rows() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let mut fx = fixture(vec![now - 3 * DAY, session_oldest], 400, true).await;
    let store = fx.hold_writes();
    let listing = hold_the_second_api_year(&fx, session_oldest).await;
    let write = store.hold_next_write(oauth_providers::STRAVA);
    listing.notify_one();
    write.reached.notified().await;
    fx.resources
        .common
        .provider_purge
        .purge_user_provider_data(
            fx.user_id,
            &fx.tenant,
            oauth_providers::SCIOTTE,
            PurgeReason::AthleteDisconnect,
        )
        .await
        .unwrap();
    assert_eq!(
        fx.state().await,
        None,
        "the session's purge took the walk's row"
    );
    fx.revoke_grant().await;
    write.gate.notify_one();
    write.landed.notified().await;
    // The step's own check follows its write; give it a few seconds.
    let deadline = Instant::now() + Duration::from_secs(5);
    while fx.rows(oauth_providers::STRAVA).await != 0 && Instant::now() < deadline {
        sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        0,
        "bug: a step whose row a purge removed left the purged grant's rows in the store"
    );
}

/// How long a step gets, once its write landed, to check its row and take
/// back whatever it owes: two reads of the ledger and the store.
const STEP_SETTLE: Duration = Duration::from_secs(1);

/// What an earlier read left of one activity: its row, the samples its
/// provider pushed, and the athlete's gear choice for it.
#[derive(Debug, PartialEq, Eq)]
struct Kept {
    row: bool,
    samples: bool,
    gear: bool,
}

/// All three of [`Kept`].
const KEPT_WHOLE: Kept = Kept {
    row: true,
    samples: true,
    gear: true,
};

impl Fixture {
    /// Store the ride `id` starting at `start` under `key` as an earlier read
    /// did, with the samples its provider pushed and the athlete's declared
    /// gear choice for it.
    async fn hold_earlier_read(&self, key: &str, id: &str, start: i64) {
        let started = Utc.timestamp_opt(start, 0).unwrap();
        let ride = ActivityBuilder::new(
            id.to_owned(),
            "Morning ride".to_owned(),
            SportType::Ride,
            started,
            3_600,
            key.to_owned(),
        )
        .distance_meters(20_000.0)
        .build();
        self.resources
            .common
            .live_activities
            .ingest(self.user_id, self.tenant, key, &[ride], Utc::now())
            .await
            .unwrap();
        let repos = &self.resources.common.repos;
        repos
            .activity_streams
            .upsert_activity_streams(&ActivityStreamsWrite {
                tenant_id: &self.tenant,
                user_id: self.user_id,
                provider: key,
                activity_id: id,
                origin: StreamOrigin::Push,
                source_ref: None,
                streams: &TimeSeriesData {
                    timestamps: vec![0, 1],
                    heart_rate: Some(vec![Some(140), Some(141)]),
                    power: None,
                    cadence: None,
                    speed: None,
                    altitude: None,
                    temperature: None,
                    gps_coordinates: None,
                    distance: None,
                },
                synced_at: Utc::now(),
            })
            .await
            .unwrap();
        repos
            .athlete_gear
            .declare_gear_attribution(
                self.user_id,
                &GearAttribution {
                    key: ActivityGearKey {
                        provider: key.to_owned(),
                        activity_id: id.to_owned(),
                    },
                    gear_id: None,
                    source: AttributionSource::Declared,
                    start_date: started,
                    distance_meters: 20_000.0,
                },
                &[],
            )
            .await
            .unwrap();
        assert_eq!(
            self.kept(key, id, start).await,
            KEPT_WHOLE,
            "the earlier read stored the ride with its samples and gear"
        );
    }

    /// What is left of the ride `id` starting at `start` under `key`.
    async fn kept(&self, key: &str, id: &str, start: i64) -> Kept {
        let repos = &self.resources.common.repos;
        let started = Utc.timestamp_opt(start, 0).unwrap();
        let gear_key = ActivityGearKey {
            provider: key.to_owned(),
            activity_id: id.to_owned(),
        };
        Kept {
            row: repos
                .activity_cache
                .get_cached_activity(self.user_id, &self.tenant, key, id)
                .await
                .unwrap()
                .is_some(),
            samples: repos
                .activity_streams
                .get_activity_streams(&self.tenant, self.user_id, key, id)
                .await
                .unwrap()
                .is_some(),
            gear: repos
                .athlete_gear
                .list_gear_attributions(
                    self.user_id,
                    started - chrono::Duration::hours(1),
                    started + chrono::Duration::hours(1),
                )
                .await
                .unwrap()
                .iter()
                .any(|attribution| attribution.key == gear_key),
        }
    }

    /// Run `sql` on the server's database.
    async fn execute(&self, sql: &str) {
        match self.resources.agent.database.backend() {
            DatabaseBackend::SQLite(sqlite) => {
                sqlx::query(sql).execute(sqlite.pool()).await.unwrap();
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(pg) => {
                sqlx::query(sql).execute(pg.pool()).await.unwrap();
            }
        }
    }

    /// Run `sql`, binding the athlete's id as `$1`, on the server's database.
    async fn execute_for_athlete(&self, sql: &str) {
        match self.resources.agent.database.backend() {
            DatabaseBackend::SQLite(sqlite) => {
                sqlx::query(sql)
                    .bind(self.user_id.to_string())
                    .execute(sqlite.pool())
                    .await
                    .unwrap();
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(pg) => {
                sqlx::query(sql)
                    .bind(self.user_id.to_string())
                    .execute(pg.pool())
                    .await
                    .unwrap();
            }
        }
    }

    /// Remove the walk's row by no purge at all, as the reaper of a row whose
    /// lease lapsed deletes it.
    async fn remove_walk_row(&self) {
        self.execute_for_athlete("DELETE FROM activity_backfill_jobs WHERE user_id = $1")
            .await;
        assert_eq!(self.state().await, None, "the walk's row is gone");
    }

    /// Make the retry a failed step owes due now: no lease holds its row and
    /// its wait is over.
    async fn retry_due_now(&self) {
        self.execute_for_athlete(
            "UPDATE activity_backfill_jobs SET leased_until_ms = 0, next_attempt_ms = 0 \
             WHERE user_id = $1",
        )
        .await;
    }

    /// The athlete's training-history rollup days, over every year the
    /// tests walk.
    async fn rollup_days(&self) -> usize {
        let (from, to) = rollup_span();
        self.resources
            .common
            .repos
            .training_history
            .get_training_history(self.tenant, self.user_id, from, to)
            .await
            .unwrap()
            .len()
    }

    /// Store a ride of `key` 100 days back, an earlier read's: the walked
    /// rides are older than the rollup's default window reaches back, and
    /// with this one the rollup has days to warm.
    async fn base_the_rollup(&self, key: &str) {
        let start = Utc::now().timestamp() - 100 * DAY;
        self.hold_earlier_read(key, &format!("8{start}"), start)
            .await;
    }

    /// Empty the athlete's training-history rollup, so a step's warm of it
    /// shows.
    async fn clear_rollup(&self) {
        let (from, to) = rollup_span();
        self.resources
            .common
            .repos
            .training_history
            .delete_training_history_range(self.tenant, self.user_id, from, to)
            .await
            .unwrap();
        assert_eq!(self.rollup_days().await, 0, "the rollup is empty");
    }

    /// Wait for a step that keeps its page to end: the last thing it writes
    /// is the training-history rollup [`Self::clear_rollup`] emptied, after
    /// any take-back. `false` when the rollup was never warmed.
    async fn rollup_warmed(&self) -> bool {
        let deadline = Instant::now() + WALK_DEADLINE;
        while self.rollup_days().await == 0 {
            if Instant::now() >= deadline {
                return false;
            }
            sleep(Duration::from_millis(50)).await;
        }
        true
    }

    /// Store a day of the grant's wellness, synced `age_days` ago.
    async fn sync_grant_wellness(&self, age_days: i64) {
        let synced_at = Utc::now() - chrono::Duration::days(age_days);
        let repos = &self.resources.common.repos;
        let source_id = repos
            .data_sources
            .upsert_data_source(
                &self.tenant,
                &DataSource {
                    id: String::new(),
                    user_id: self.user_id.to_string(),
                    provider: oauth_providers::STRAVA.to_owned(),
                    device_model: None,
                    software_version: None,
                    source: None,
                    device_type: DeviceType::Unknown,
                    original_source_name: None,
                },
            )
            .await
            .unwrap();
        repos
            .daily_activity
            .upsert_daily_activity(
                &self.tenant,
                &StoredDailyActivity {
                    id: String::new(),
                    user_id: self.user_id.to_string(),
                    data_source_id: source_id,
                    date: synced_at.date_naive(),
                    steps: Some(9_000),
                    active_kcal: None,
                    total_kcal: None,
                    intensity_minutes: None,
                    source_name: oauth_providers::STRAVA.to_owned(),
                    recorded_at: synced_at,
                },
                WellnessWriteMode::PartialSummary,
                synced_at,
            )
            .await
            .unwrap();
    }

    /// The reasons of the purges of `key` attested for the athlete.
    async fn attested_for_athlete(&self, key: &str) -> Vec<String> {
        let athlete = self.user_id.to_string();
        self.resources
            .common
            .repos
            .provider_data
            .list_provider_data_purges(key, 20)
            .await
            .unwrap()
            .into_iter()
            .filter(|record| record.user_id.as_deref() == Some(athlete.as_str()))
            .map(|record| record.reason)
            .collect()
    }
}

/// The days the rollup checks look over: every year the tests walk.
fn rollup_span() -> (NaiveDate, NaiveDate) {
    let today = Utc::now().date_naive();
    (
        today - chrono::Duration::days(12 * 365),
        today + chrono::Duration::days(1),
    )
}

/// Start the walk, and wait until its session's first step reached the
/// held `write`.
async fn start_with_the_first_step_held(fx: &Fixture, write: &Hold) {
    assert!(
        start_history_walk(
            &fx.runtime(),
            fx.user_id,
            fx.tenant,
            oauth_providers::STRAVA,
            WalkTrigger::Connected
        )
        .await,
        "the walk of the session starts"
    );
    timeout(WALK_DEADLINE, write.reached.notified())
        .await
        .expect("the session's first step never wrote");
}

/// While renamed, every read of a walk's row fails (each names
/// `walk_backend`), and the lease writes, which never name it, still land.
const BREAK_WALK_ROW_READS: &str =
    "ALTER TABLE activity_backfill_jobs RENAME COLUMN walk_backend TO walk_backend_unread";

/// Undoes [`BREAK_WALK_ROW_READS`].
const MEND_WALK_ROW_READS: &str =
    "ALTER TABLE activity_backfill_jobs RENAME COLUMN walk_backend_unread TO walk_backend";

/// The grant's second year below the session holds a ride an earlier read
/// of the grant stored, with its samples and gear; the athlete then drops
/// the session alone while a step of the walk's API phase writes that year.
/// The session's purge takes the walk's row, but the grant was never
/// purged: the step keeps the page it wrote, the earlier ride whole beside
/// it.
#[tokio::test]
async fn a_session_purge_landing_while_a_step_writes_keeps_the_grants_page() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let second_year = session_oldest - 3 * YEAR / 2;
    let earlier = second_year.to_string();
    let mut fx = fixture(vec![now - 3 * DAY, session_oldest], 400, true).await;
    let store = fx.hold_writes();
    let listing = hold_the_second_api_year(&fx, session_oldest).await;
    fx.hold_earlier_read(oauth_providers::STRAVA, &earlier, second_year)
        .await;
    let write = store.hold_next_write(oauth_providers::STRAVA);
    listing.notify_one();
    write.reached.notified().await;
    fx.resources
        .common
        .provider_purge
        .purge_user_provider_data(
            fx.user_id,
            &fx.tenant,
            oauth_providers::SCIOTTE,
            PurgeReason::AthleteDisconnect,
        )
        .await
        .unwrap();
    assert_eq!(
        fx.state().await,
        None,
        "the session's purge took the walk's row"
    );
    fx.base_the_rollup(oauth_providers::STRAVA).await;
    fx.clear_rollup().await;
    write.gate.notify_one();
    let warmed = fx.rollup_warmed().await;
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        3,
        "bug: a step whose session alone was purged took the never-purged grant's page back"
    );
    assert_eq!(
        fx.kept(oauth_providers::STRAVA, &earlier, second_year)
            .await,
        KEPT_WHOLE,
        "bug: the step took back a ride an earlier read stored, with its samples or gear"
    );
    assert!(
        warmed,
        "bug: a step that kept the grant's page never warmed the training-history rollup over it"
    );
}

/// The athlete drops the session alone while a step of the walk's API phase
/// writes the grant's second year, and the hourly retention sweep expires
/// the grant's wellness in that same moment. The session's purge took the
/// walk's row; the wellness expiry deleted none of the grant's activities,
/// and attests as much: the step keeps the page it wrote.
#[tokio::test]
async fn a_wellness_expiry_landing_while_a_step_writes_takes_nothing_back() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let mut fx = fixture(vec![now - 3 * DAY, session_oldest], 400, true).await;
    fx.sync_grant_wellness(40).await;
    let store = fx.hold_writes();
    let listing = hold_the_second_api_year(&fx, session_oldest).await;
    let write = store.hold_next_write(oauth_providers::STRAVA);
    listing.notify_one();
    write.reached.notified().await;
    fx.resources
        .common
        .provider_purge
        .purge_user_provider_data(
            fx.user_id,
            &fx.tenant,
            oauth_providers::SCIOTTE,
            PurgeReason::AthleteDisconnect,
        )
        .await
        .unwrap();
    assert_eq!(
        fx.state().await,
        None,
        "the session's purge took the walk's row"
    );
    let expiry = fx
        .resources
        .common
        .repos
        .provider_data
        .expire_provider_wellness(
            oauth_providers::STRAVA,
            Utc::now() - chrono::Duration::days(30),
        )
        .await
        .unwrap();
    assert_eq!(expiry.scopes, 1, "the grant's wellness expired");
    fx.base_the_rollup(oauth_providers::STRAVA).await;
    fx.clear_rollup().await;
    write.gate.notify_one();
    let warmed = fx.rollup_warmed().await;
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        3,
        "bug: a wellness expiry of the grant's key took back the activities its step added"
    );
    assert_eq!(
        fx.attested_for_athlete(oauth_providers::STRAVA).await,
        ["wellness_expired"],
        "bug: the wellness expiry attests as an activity expiry"
    );
    assert!(warmed, "the step that kept its page warmed the rollup");
}

/// How long after a step resumed from its write the tests mend a read they
/// broke: past the step's first check and its first retry (250 ms after),
/// well before its last retry (3.25 s after).
const MID_RETRY: Duration = Duration::from_millis(500);

/// Longer than every retry of a step's check after its write takes
/// (250 ms, 1 s and 2 s of waiting, and four reads).
const RETRIES_SPENT: Duration = Duration::from_secs(6);

impl Fixture {
    /// Run `mend` once the step that just resumed from its write found its
    /// check after it unreadable, and before its retries are spent.
    async fn mend_mid_retry(&self, mend: &str) {
        sleep(MID_RETRY).await;
        self.execute(mend).await;
    }
}

/// The ledger cannot be read once a step of the walk's API phase wrote the
/// grant's second year, a year holding a ride an earlier read had stored
/// with its samples and gear, and a ride no read stored yet, and it stays
/// unreadable through every retry of the step's check. The step takes
/// nothing back: it gives its row back with its retry due a lease later, as
/// any failed step does. Nothing was purged: the retry reads the same year
/// again and lands it over the rows that stayed.
#[tokio::test]
async fn a_ledger_unread_after_a_steps_write_takes_nothing_back_and_fails() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let second_year = session_oldest - 3 * YEAR / 2;
    let earlier = second_year.to_string();
    let mut fx = fixture(vec![now - 3 * DAY, session_oldest], 400, true).await;
    let store = fx.hold_writes();
    let listing = hold_the_second_api_year(&fx, session_oldest).await;
    fx.strava.serve(vec![
        session_oldest - YEAR / 2,
        second_year + DAY,
        second_year,
        session_oldest - 5 * YEAR / 2,
    ]);
    fx.hold_earlier_read(oauth_providers::STRAVA, &earlier, second_year)
        .await;
    let write = store.pause_after_next_write(oauth_providers::STRAVA);
    listing.notify_one();
    write.landed.notified().await;
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        3,
        "the step's write added the year's new ride"
    );
    fx.execute(BREAK_WALK_ROW_READS).await;
    let failed_at = Utc::now().timestamp_millis();
    write.resume.as_ref().unwrap().notify_one();
    let deadline = Instant::now() + WALK_DEADLINE;
    while !fx.idle().await {
        assert!(
            Instant::now() < deadline,
            "the step never gave its row back"
        );
        sleep(Duration::from_millis(50)).await;
    }
    fx.execute(MEND_WALK_ROW_READS).await;
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        3,
        "bug: a step whose check after its write stayed unreadable took back rows no purge reached"
    );
    assert_eq!(
        fx.kept(oauth_providers::STRAVA, &earlier, second_year)
            .await,
        KEPT_WHOLE,
        "bug: a step that could not read its row took back a ride an earlier read stored"
    );
    assert_eq!(
        fx.state().await,
        Some(BackfillJobState::Owed),
        "the walk stays owed for the step's retry"
    );
    let ledger = &fx.resources.common.repos.activity_backfill_jobs;
    let early = ledger
        .claim_stale_backfill_jobs(Fixture::claim_at(failed_at + 60_000))
        .await
        .unwrap();
    assert!(
        early.is_empty(),
        "bug: the step is retried before a whole lease passed: {early:?}"
    );
    let lease = i64::try_from(BACKFILL_JOB_LEASE.as_millis()).unwrap();
    let due = ledger
        .claim_stale_backfill_jobs(Fixture::claim_at(
            Utc::now().timestamp_millis() + lease + 2_000,
        ))
        .await
        .unwrap();
    assert_eq!(
        due.len(),
        1,
        "bug: the step's retry is not due once a whole lease passed"
    );
    let failed_listing = fx.strava.listings().last().copied();
    let listed = fx.strava.listing_count();
    fx.retry_due_now().await;
    let deadline = Instant::now() + WALK_DEADLINE;
    while !(fx.strava.listing_count() > listed && fx.idle().await) {
        assert!(
            Instant::now() < deadline,
            "bug: the step's retry never read the page again"
        );
        fx.sweep(false).await;
    }
    assert_eq!(
        fx.strava.listings().last().copied(),
        failed_listing,
        "the retry read the same year again"
    );
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        3,
        "the retry landed the year over the rows that stayed"
    );
    assert_eq!(
        fx.kept(oauth_providers::STRAVA, &earlier, second_year)
            .await,
        KEPT_WHOLE,
        "the retry kept the ride an earlier read stored"
    );
}

/// The grant's second year holds a ride an earlier read stored and a ride
/// no read stored yet, and the grant is revoked while a step of the walk's
/// API phase writes that year: the purge deleted the earlier ride before
/// the write landed both. The ledger cannot be read for a moment once the
/// write landed; the step's retry reads it, finds its phase purged, and
/// takes its whole page back, the ride it added and the one it found
/// stored: nothing of the purged grant is left.
#[tokio::test]
async fn a_ledger_unread_for_a_moment_after_a_steps_write_takes_back_what_a_purge_reached() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let second_year = session_oldest - 3 * YEAR / 2;
    let mut fx = fixture(vec![now - 3 * DAY, session_oldest], 400, true).await;
    let store = fx.hold_writes();
    let listing = hold_the_second_api_year(&fx, session_oldest).await;
    fx.strava.serve(vec![
        session_oldest - YEAR / 2,
        second_year + DAY,
        second_year,
        session_oldest - 5 * YEAR / 2,
    ]);
    fx.hold_earlier_read(
        oauth_providers::STRAVA,
        &second_year.to_string(),
        second_year,
    )
    .await;
    let write = store.hold_and_pause_next_write(oauth_providers::STRAVA);
    listing.notify_one();
    write.reached.notified().await;
    fx.revoke_grant().await;
    write.gate.notify_one();
    write.landed.notified().await;
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        2,
        "the step's write added the year's new ride and put back the ride the purge deleted"
    );
    fx.execute(BREAK_WALK_ROW_READS).await;
    write.resume.as_ref().unwrap().notify_one();
    fx.mend_mid_retry(MEND_WALK_ROW_READS).await;
    fx.step_stopped().await;
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        0,
        "bug: a step whose check after its write failed once left what the grant's purge reached"
    );
}

/// While renamed, every read of the purge attestations fails.
const BREAK_PURGE_READS: &str =
    "ALTER TABLE provider_data_purges RENAME COLUMN reason TO reason_unread";

/// Undoes [`BREAK_PURGE_READS`].
const MEND_PURGE_READS: &str =
    "ALTER TABLE provider_data_purges RENAME COLUMN reason_unread TO reason";

/// The athlete drops both connections while a step of the walk's API phase
/// writes the grant's second year, a year holding a ride an earlier read
/// stored and a ride no read stored yet, and the purge attestations cannot
/// be read for a moment once the write landed. The step finds its row gone;
/// its retry reads the grant's purge and takes its whole page back, the
/// ride it added and the one it found stored: nothing of the purged grant
/// is left.
#[tokio::test]
async fn a_purge_attestation_unread_for_a_moment_takes_back_what_the_purge_reached() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let second_year = session_oldest - 3 * YEAR / 2;
    let mut fx = fixture(vec![now - 3 * DAY, session_oldest], 400, true).await;
    let store = fx.hold_writes();
    let listing = hold_the_second_api_year(&fx, session_oldest).await;
    fx.strava.serve(vec![
        session_oldest - YEAR / 2,
        second_year + DAY,
        second_year,
        session_oldest - 5 * YEAR / 2,
    ]);
    fx.hold_earlier_read(
        oauth_providers::STRAVA,
        &second_year.to_string(),
        second_year,
    )
    .await;
    let write = store.hold_and_pause_next_write(oauth_providers::STRAVA);
    listing.notify_one();
    write.reached.notified().await;
    fx.resources
        .common
        .provider_purge
        .purge_user_provider_data(
            fx.user_id,
            &fx.tenant,
            oauth_providers::SCIOTTE,
            PurgeReason::AthleteDisconnect,
        )
        .await
        .unwrap();
    assert_eq!(
        fx.state().await,
        None,
        "the session's purge took the walk's row"
    );
    fx.revoke_grant().await;
    write.gate.notify_one();
    write.landed.notified().await;
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        2,
        "the step's write added the year's new ride and put back the ride the grant's purge deleted"
    );
    fx.execute(BREAK_PURGE_READS).await;
    write.resume.as_ref().unwrap().notify_one();
    fx.mend_mid_retry(MEND_PURGE_READS).await;
    let deadline = Instant::now() + WALK_DEADLINE;
    while fx.rows(oauth_providers::STRAVA).await != 0 && Instant::now() < deadline {
        sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        0,
        "bug: a step whose purge read failed once left what the grant's purge reached"
    );
}

/// The walk's row is removed by no purge while the session's first step
/// writes, and the purge attestations cannot be read for a moment once the
/// write landed. The step's retry reads them: no purge of the session's key
/// landed, so it keeps its page and warms the training-history rollup.
#[tokio::test]
async fn a_purge_attestation_unread_for_a_moment_with_no_purge_keeps_the_page() {
    let now = Utc::now().timestamp();
    let mut fx = fixture(vec![now - 3 * DAY, now - 2 * YEAR], 400, true).await;
    let store = fx.hold_writes();
    let write = store.hold_and_pause_next_write(oauth_providers::SCIOTTE);
    start_with_the_first_step_held(&fx, &write).await;
    fx.remove_walk_row().await;
    fx.base_the_rollup(oauth_providers::SCIOTTE).await;
    fx.clear_rollup().await;
    write.gate.notify_one();
    write.landed.notified().await;
    fx.execute(BREAK_PURGE_READS).await;
    write.resume.as_ref().unwrap().notify_one();
    fx.mend_mid_retry(MEND_PURGE_READS).await;
    let warmed = fx.rollup_warmed().await;
    assert_eq!(
        fx.rows(oauth_providers::SCIOTTE).await,
        3,
        "bug: a step whose purge read failed once took back the page of a key no purge reached"
    );
    assert!(
        warmed,
        "bug: a step whose retry read no purge never kept its page"
    );
}

/// The walk's row is removed by no purge while the session's first step
/// writes, after another writer stored one of the step's rides with its
/// samples and gear, and the purge attestations stay unreadable through
/// every retry of the step's check. The step cannot tell whether a purge
/// landed and takes nothing back: its page and the other writer's ride stay
/// whole, and it fails without warming the rollup.
#[tokio::test]
async fn a_purge_attestation_unread_through_every_retry_deletes_nothing() {
    let now = Utc::now().timestamp();
    let (recent, session_oldest) = (now - 3 * DAY, now - 2 * YEAR);
    let other = format!("9{recent}");
    let mut fx = fixture(vec![recent, session_oldest], 400, true).await;
    let store = fx.hold_writes();
    let write = store.hold_and_pause_next_write(oauth_providers::SCIOTTE);
    start_with_the_first_step_held(&fx, &write).await;
    fx.hold_earlier_read(oauth_providers::SCIOTTE, &other, recent)
        .await;
    fx.remove_walk_row().await;
    fx.base_the_rollup(oauth_providers::SCIOTTE).await;
    fx.clear_rollup().await;
    write.gate.notify_one();
    write.landed.notified().await;
    fx.execute(BREAK_PURGE_READS).await;
    write.resume.as_ref().unwrap().notify_one();
    sleep(RETRIES_SPENT).await;
    fx.execute(MEND_PURGE_READS).await;
    assert_eq!(
        fx.rows(oauth_providers::SCIOTTE).await,
        3,
        "bug: a step whose purge reads stayed unreadable deleted rows of a key no purge reached"
    );
    assert_eq!(
        fx.kept(oauth_providers::SCIOTTE, &other, recent).await,
        KEPT_WHOLE,
        "bug: a step whose purge reads stayed unreadable deleted a ride another writer stored"
    );
    assert_eq!(
        fx.rollup_days().await,
        0,
        "bug: a step whose purge reads stayed unreadable kept its page instead of failing"
    );
}

/// The operator terminates Strava after a step of the walk's API phase
/// found its row owed and before its write of the grant's second year
/// landed: the write lands after the termination deleted the provider's
/// rows. The step sees the walk settled as its session walked alone once it
/// wrote, and takes its page back: nothing of the terminated provider is
/// left in the store.
#[tokio::test]
async fn a_termination_landing_while_a_step_writes_leaves_none_of_its_rows() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let mut fx = fixture(vec![now - 3 * DAY, session_oldest], 400, true).await;
    let store = fx.hold_writes();
    let listing = hold_the_second_api_year(&fx, session_oldest).await;
    let write = store.hold_next_write(oauth_providers::STRAVA);
    listing.notify_one();
    write.reached.notified().await;
    fx.terminate_strava().await;
    write.gate.notify_one();
    fx.step_stopped().await;
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        0,
        "bug: a step's write that landed after its provider's termination left the terminated provider's rows in the store"
    );
    assert_eq!(
        fx.rows(oauth_providers::SCIOTTE).await,
        2,
        "the session's rows stay with the session"
    );
}

/// The grant's second year holds a ride an earlier read of the grant
/// stored, and the grant is revoked while a step of the walk's API phase
/// writes that year: the purge deleted the earlier ride before the write
/// landed it again. The whole page goes back, the ride the step found
/// already stored included: nothing of the purged grant is left.
#[tokio::test]
async fn a_grant_purge_landing_while_a_step_writes_takes_back_its_whole_page() {
    let now = Utc::now().timestamp();
    let session_oldest = now - 2 * YEAR;
    let second_year = session_oldest - 3 * YEAR / 2;
    let mut fx = fixture(vec![now - 3 * DAY, session_oldest], 400, true).await;
    let store = fx.hold_writes();
    let listing = hold_the_second_api_year(&fx, session_oldest).await;
    fx.hold_earlier_read(
        oauth_providers::STRAVA,
        &second_year.to_string(),
        second_year,
    )
    .await;
    let write = store.hold_next_write(oauth_providers::STRAVA);
    listing.notify_one();
    write.reached.notified().await;
    fx.revoke_grant().await;
    write.gate.notify_one();
    fx.step_stopped().await;
    assert_eq!(
        fx.rows(oauth_providers::STRAVA).await,
        0,
        "bug: a ride the purge deleted and the step's write stored again is left in the store"
    );
}

/// The session's first step reads its two rides; the recent one an earlier
/// read already stored, with its samples and gear. The retention expiry of
/// an old session ride settles while the step writes, resetting the
/// session's sync state, the walk's row with it. The expiry deleted none of
/// the page: the step takes back only the ride it added, and the earlier
/// ride stays whole.
#[tokio::test]
async fn an_expiry_landing_while_a_step_writes_takes_back_only_what_it_added() {
    let now = Utc::now().timestamp();
    let (recent, session_oldest) = (now - 3 * DAY, now - 2 * YEAR);
    let earlier = format!("9{recent}");
    let mut fx = fixture(vec![recent, session_oldest], 400, true).await;
    let store = fx.hold_writes();
    fx.hold_earlier_read(oauth_providers::SCIOTTE, &earlier, recent)
        .await;
    let write = store.hold_next_write(oauth_providers::SCIOTTE);
    assert!(
        start_history_walk(
            &fx.runtime(),
            fx.user_id,
            fx.tenant,
            oauth_providers::STRAVA,
            WalkTrigger::Connected
        )
        .await,
        "the walk of the session starts"
    );
    timeout(WALK_DEADLINE, write.reached.notified())
        .await
        .expect("the session's first step never wrote");
    fx.resources
        .common
        .repos
        .provider_data
        .expire_provider_activities(
            fx.user_id,
            &fx.tenant,
            oauth_providers::SCIOTTE,
            &["9100000000"],
        )
        .await
        .unwrap();
    assert_eq!(fx.state().await, None, "the expiry reset the walk's row");
    write.gate.notify_one();
    write.landed.notified().await;
    let deadline = Instant::now() + STEP_SETTLE;
    while fx.rows(oauth_providers::SCIOTTE).await != 1 && Instant::now() < deadline {
        sleep(Duration::from_millis(50)).await;
    }
    sleep(STEP_SETTLE).await;
    assert_eq!(
        fx.kept(oauth_providers::SCIOTTE, &earlier, recent).await,
        KEPT_WHOLE,
        "bug: an expiry that deleted none of the page took back a ride an earlier read stored"
    );
    assert_eq!(
        fx.rows(oauth_providers::SCIOTTE).await,
        1,
        "bug: the step left the ride it added after the expiry reset its row"
    );
}

/// The session's first step reads its two rides, the recent one an earlier
/// read already stored. Once the step read what the store held, and while
/// its write waits, another writer stores the older ride under the session's
/// key with its samples and gear; then the retention expiry resets the
/// walk's row. The step's write inserted nothing the store did not hold by
/// then, so it takes nothing back: the other writer's ride stays whole.
#[tokio::test]
async fn an_expiry_never_takes_back_a_ride_another_writer_stored_during_the_step() {
    let now = Utc::now().timestamp();
    let (recent, session_oldest) = (now - 3 * DAY, now - 2 * YEAR);
    let (earlier, concurrent) = (format!("9{recent}"), format!("9{session_oldest}"));
    let mut fx = fixture(vec![recent, session_oldest], 400, true).await;
    let store = fx.hold_writes();
    fx.hold_earlier_read(oauth_providers::SCIOTTE, &earlier, recent)
        .await;
    let write = store.hold_next_write(oauth_providers::SCIOTTE);
    start_with_the_first_step_held(&fx, &write).await;
    fx.hold_earlier_read(oauth_providers::SCIOTTE, &concurrent, session_oldest)
        .await;
    fx.resources
        .common
        .repos
        .provider_data
        .expire_provider_activities(
            fx.user_id,
            &fx.tenant,
            oauth_providers::SCIOTTE,
            &["9100000000"],
        )
        .await
        .unwrap();
    assert_eq!(fx.state().await, None, "the expiry reset the walk's row");
    write.gate.notify_one();
    write.landed.notified().await;
    sleep(STEP_SETTLE).await;
    assert_eq!(
        fx.kept(oauth_providers::SCIOTTE, &concurrent, session_oldest)
            .await,
        KEPT_WHOLE,
        "bug: the step took back a ride another writer stored under its key while it wrote"
    );
    assert_eq!(
        fx.kept(oauth_providers::SCIOTTE, &earlier, recent).await,
        KEPT_WHOLE,
        "bug: the step took back a ride an earlier read stored"
    );
}

/// The same concurrent writer, beside a ride the cache served before the
/// store kept the session's reads: the step's write inserts that ride into
/// the store, which once made up for the concurrent ride it did not insert.
/// The step counts against what the store recorded, not what it served, so
/// it still takes nothing back and the other writer's ride stays whole.
#[tokio::test]
async fn a_cache_only_ride_never_hides_a_concurrent_writers_ride_from_an_expiry() {
    let now = Utc::now().timestamp();
    let (recent, session_oldest) = (now - 3 * DAY, now - 2 * YEAR);
    let (cached, concurrent) = (format!("9{recent}"), format!("9{session_oldest}"));
    let mut fx = fixture(vec![recent, session_oldest], 400, true).await;
    let store = fx.hold_writes();
    let ride = ActivityBuilder::new(
        cached.clone(),
        "Morning ride".to_owned(),
        SportType::Ride,
        Utc.timestamp_opt(recent, 0).unwrap(),
        3_600,
        oauth_providers::SCIOTTE.to_owned(),
    )
    .build();
    fx.resources
        .common
        .repos
        .activity_cache
        .upsert_activities(fx.user_id, &fx.tenant, oauth_providers::SCIOTTE, &[ride])
        .await
        .unwrap();
    let write = store.hold_next_write(oauth_providers::SCIOTTE);
    start_with_the_first_step_held(&fx, &write).await;
    fx.hold_earlier_read(oauth_providers::SCIOTTE, &concurrent, session_oldest)
        .await;
    fx.resources
        .common
        .repos
        .provider_data
        .expire_provider_activities(
            fx.user_id,
            &fx.tenant,
            oauth_providers::SCIOTTE,
            &["9100000000"],
        )
        .await
        .unwrap();
    assert_eq!(fx.state().await, None, "the expiry reset the walk's row");
    write.gate.notify_one();
    write.landed.notified().await;
    sleep(STEP_SETTLE).await;
    assert_eq!(
        fx.kept(oauth_providers::SCIOTTE, &concurrent, session_oldest)
            .await,
        KEPT_WHOLE,
        "bug: a cache-only ride the step inserted hid the concurrent writer's ride it then took back"
    );
    assert_eq!(
        fx.rows(oauth_providers::SCIOTTE).await,
        2,
        "bug: the step took back a ride the cache served before its write"
    );
}

/// The session's first step reads its two rides; the athlete deleted the
/// recent one, so the store refuses it. The retention expiry resets the
/// walk's row while the step writes: the refused ride counts as the step's
/// own, so the step still takes back the ride it added, and nothing of its
/// page is left.
#[tokio::test]
async fn an_expiry_takes_back_what_the_step_added_beside_a_ride_the_athlete_deleted() {
    let now = Utc::now().timestamp();
    let (recent, session_oldest) = (now - 3 * DAY, now - 2 * YEAR);
    let mut fx = fixture(vec![recent, session_oldest], 400, true).await;
    let store = fx.hold_writes();
    assert!(
        fx.resources
            .common
            .live_activities
            .erase(
                fx.user_id,
                fx.tenant,
                oauth_providers::SCIOTTE,
                &format!("9{recent}")
            )
            .await
            .is_ok(),
        "the athlete deletes the recent ride"
    );
    let write = store.hold_next_write(oauth_providers::SCIOTTE);
    start_with_the_first_step_held(&fx, &write).await;
    fx.resources
        .common
        .repos
        .provider_data
        .expire_provider_activities(
            fx.user_id,
            &fx.tenant,
            oauth_providers::SCIOTTE,
            &["9100000000"],
        )
        .await
        .unwrap();
    assert_eq!(fx.state().await, None, "the expiry reset the walk's row");
    write.gate.notify_one();
    write.landed.notified().await;
    let deadline = Instant::now() + STEP_SETTLE;
    while fx.rows(oauth_providers::SCIOTTE).await != 0 && Instant::now() < deadline {
        sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        fx.rows(oauth_providers::SCIOTTE).await,
        0,
        "bug: a ride the athlete deleted kept the step from taking back the ride it added"
    );
}

/// The session's first step reads its two rides, the recent one an earlier
/// read already stored with its samples and gear, and the walk's row is
/// removed while the step writes by no purge at all (the reaper of a row
/// whose lease lapsed deletes it the same way). Nothing attests a purge of
/// the session's key: the step keeps its page, warms the training-history
/// rollup over it, and ends.
#[tokio::test]
async fn a_row_removed_by_no_purge_while_a_step_writes_keeps_its_page() {
    let now = Utc::now().timestamp();
    let (recent, session_oldest) = (now - 3 * DAY, now - 2 * YEAR);
    let earlier = format!("9{recent}");
    let mut fx = fixture(vec![recent, session_oldest], 400, true).await;
    let store = fx.hold_writes();
    fx.hold_earlier_read(oauth_providers::SCIOTTE, &earlier, recent)
        .await;
    let write = store.hold_next_write(oauth_providers::SCIOTTE);
    start_with_the_first_step_held(&fx, &write).await;
    fx.remove_walk_row().await;
    fx.base_the_rollup(oauth_providers::SCIOTTE).await;
    fx.clear_rollup().await;
    write.gate.notify_one();
    let warmed = fx.rollup_warmed().await;
    assert_eq!(
        fx.rows(oauth_providers::SCIOTTE).await,
        3,
        "bug: a step whose row no purge removed took its page back"
    );
    assert_eq!(
        fx.kept(oauth_providers::SCIOTTE, &earlier, recent).await,
        KEPT_WHOLE,
        "bug: a step whose row no purge removed took back a ride an earlier read stored"
    );
    assert!(
        warmed,
        "bug: a step that kept its page never warmed the training-history rollup over it"
    );
}

/// The session's activities expired a moment before the walk started, so
/// that expiry's attestation sits inside the window the step reads purges
/// from; then the walk's row is removed by no purge while the session's
/// first step writes. The step subtracts what it read before its first
/// check: no purge landed during its write, and it keeps its page.
#[tokio::test]
async fn a_purge_attested_before_a_steps_check_takes_nothing_back() {
    let now = Utc::now().timestamp();
    let (recent, session_oldest) = (now - 3 * DAY, now - 2 * YEAR);
    let earlier = format!("9{recent}");
    let mut fx = fixture(vec![recent, session_oldest], 400, true).await;
    fx.resources
        .common
        .repos
        .provider_data
        .expire_provider_activities(
            fx.user_id,
            &fx.tenant,
            oauth_providers::SCIOTTE,
            &["9100000000"],
        )
        .await
        .unwrap();
    let store = fx.hold_writes();
    fx.hold_earlier_read(oauth_providers::SCIOTTE, &earlier, recent)
        .await;
    let write = store.hold_next_write(oauth_providers::SCIOTTE);
    start_with_the_first_step_held(&fx, &write).await;
    fx.remove_walk_row().await;
    fx.base_the_rollup(oauth_providers::SCIOTTE).await;
    fx.clear_rollup().await;
    write.gate.notify_one();
    let warmed = fx.rollup_warmed().await;
    assert_eq!(
        fx.rows(oauth_providers::SCIOTTE).await,
        3,
        "bug: an expiry attested before the step's first check took back the ride it added"
    );
    assert_eq!(
        fx.kept(oauth_providers::SCIOTTE, &earlier, recent).await,
        KEPT_WHOLE,
        "the ride an earlier read stored stays whole"
    );
    assert!(warmed, "the step that kept its page warmed the rollup");
}

/// The operator terminates the session's own provider while the walk's
/// first session step writes its page, a ride an earlier read stored among
/// it. The termination deleted the walk's row, filed under the terminated
/// key, and attests a purge reaching every athlete, naming none. The step
/// reads that attestation and takes its whole page back: nothing of the
/// terminated provider is left in the store.
#[tokio::test]
async fn a_termination_of_the_steps_own_key_takes_back_its_whole_page() {
    let now = Utc::now().timestamp();
    let (recent, session_oldest) = (now - 3 * DAY, now - 2 * YEAR);
    let earlier = format!("9{recent}");
    let mut fx = fixture(vec![recent, session_oldest], 400, true).await;
    let store = fx.hold_writes();
    fx.hold_earlier_read(oauth_providers::SCIOTTE, &earlier, recent)
        .await;
    let write = store.hold_next_write(oauth_providers::SCIOTTE);
    start_with_the_first_step_held(&fx, &write).await;
    fx.resources
        .common
        .provider_purge
        .purge_provider_data(oauth_providers::SCIOTTE)
        .await
        .unwrap();
    assert_eq!(
        (fx.state().await, fx.rows(oauth_providers::SCIOTTE).await),
        (None, 0),
        "the termination took the walk's row and the session's rows"
    );
    write.gate.notify_one();
    write.landed.notified().await;
    let deadline = Instant::now() + WALK_DEADLINE;
    while fx.rows(oauth_providers::SCIOTTE).await != 0 && Instant::now() < deadline {
        sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        fx.rows(oauth_providers::SCIOTTE).await,
        0,
        "bug: a step whose own key was terminated left the terminated provider's rows in the store"
    );
}
