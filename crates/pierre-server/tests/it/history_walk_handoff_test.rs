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
use crate::history_walk_test::age_job_rows;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::extract::RawQuery;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Datelike, TimeZone, Utc};
use pierre_config::environment::ProviderEndpointOverride;
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{ConnectionType, TenantId};
use pierre_database::repositories::{BackfillJobKind, BackfillJobState, JobProgress};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::activity_backfill::history_walk::start_history_walk;
use pierre_tool_runtime::activity_backfill_resume::resume_backfill_jobs;
use pierre_tool_runtime::runtime::{
    BackfillCompletion, BackfillNotifier, HistoryWalkCompletion, ReauthRetry, ToolRuntime,
    WalkTrigger,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::time::sleep;
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
        .months_total
        .expect("the grant's profile names the floor");
    assert!(
        (70..=72).contains(&total),
        "bug: the walk's length runs from now to the account's creation: {progress:?}"
    );
    assert!(
        progress.months_done <= total && progress.months_done >= total - 4,
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
/// its oldest row, and the end of that phase is told too.
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
        told[1].2, 3,
        "bug: the post counts the whole history the walk holds"
    );
    assert!(
        !walk(WalkTrigger::Connected).await,
        "bug: a walk whose API phase ran is not reopened again"
    );
}
