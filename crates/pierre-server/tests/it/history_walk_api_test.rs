// ABOUTME: A history walk through an API grant: a year a window, the Strava account's creation as its floor, a budget pause
// ABOUTME: And a finished walk never re-asked by a depth ask

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! History walks through an API grant (carnet#1482).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// A Strava athlete with only the API grant is walked through it: newest
// first, a year a window, the first step reading the athlete's profile for
// the account's creation, which is the walk's floor and gives its length. A
// spent request budget pauses the walk with its cursor and its attempt; the
// sweep resumes it in the next window. With no creation date the walk ends
// after two empty years. A depth ask never walks a source whose walk already
// ended. The source order the walk reads is pinned in
// `history_source_order_test`.
//
// Strava is a loopback stand-in (a test double, per the repo's mock rule)
// that serves the history inside each read's `after`/`before` and counts
// every request.
#![cfg(all(
    feature = "provider-strava",
    feature = "provider-garmin",
    feature = "health-sync"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::history_walk_test::{age_job_rows, paused_retries_due};

use std::collections::HashMap;
use std::mem;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{Duration as ChronoDuration, TimeZone, Utc};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{ConnectionType, TenantId, UserOAuthToken};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::{
    BackfillJobClaim, BackfillJobKind, BackfillJobState, JobProgress,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::request_budget::{ProviderRateLimiter, ONE_DAY};
use pierre_providers::sciotte_provider::startup_client;
use pierre_tool_runtime::activity_backfill::history_walk::start_history_walk;
use pierre_tool_runtime::activity_backfill_resume::resume_backfill_jobs;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::runtime::WalkTrigger;
use secrecy::SecretString;
use serde_json::{json, Value};
use sqlx::Row;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

/// Seconds in a day.
const DAY: i64 = 86_400;

/// How long a walk gets to end before the test gives up.
const WALK_DEADLINE: Duration = Duration::from_secs(20);

/// What the Strava stand-in serves and saw.
#[derive(Default)]
pub(crate) struct MockStrava {
    /// Activity starts (unix seconds) the athlete's history holds.
    starts: Mutex<Vec<i64>>,
    /// When the account was created, if the profile says.
    created_at: Mutex<Option<i64>>,
    /// Every listing's `after` and `before`.
    listings: Mutex<Vec<(Option<i64>, Option<i64>)>>,
    /// Every listing's `per_page`.
    page_sizes: Mutex<Vec<usize>>,
    /// Profile reads.
    profiles: Mutex<usize>,
    /// Every activity detail read's id.
    details: Mutex<Vec<i64>>,
    /// The gate the next listing waits on once it is counted, when one is set.
    hold: Mutex<Option<Arc<Notify>>>,
    /// Raised when a listing reached the gate.
    held: Notify,
    /// Whether the held listing fails once the gate lets it go.
    fail_held: Mutex<bool>,
}

impl MockStrava {
    /// Serve `starts` as the athlete's history.
    pub(crate) fn serve(&self, starts: Vec<i64>) {
        *self.starts.lock().unwrap() = starts;
    }

    /// How many activity listings the stand-in has answered.
    pub(crate) fn listing_count(&self) -> usize {
        self.listings.lock().unwrap().len()
    }

    /// How many activity detail reads the stand-in has answered.
    pub(crate) fn detail_count(&self) -> usize {
        self.details.lock().unwrap().len()
    }

    /// Name `created` (unix seconds) as the account's creation on the profile.
    pub(crate) fn created(&self, created: i64) {
        *self.created_at.lock().unwrap() = Some(created);
    }

    /// Every listing's `after` and `before`, in the order they came.
    pub(crate) fn listings(&self) -> Vec<(Option<i64>, Option<i64>)> {
        self.listings.lock().unwrap().clone()
    }

    /// Every listing's `per_page`, in the order they came.
    pub(crate) fn page_sizes(&self) -> Vec<usize> {
        self.page_sizes.lock().unwrap().clone()
    }

    /// How many profile reads the stand-in has answered.
    pub(crate) fn profile_count(&self) -> usize {
        *self.profiles.lock().unwrap()
    }

    /// Hold the next listing, once counted, until the returned gate is
    /// notified: a walk step caught mid-request.
    pub(crate) fn hold_next_listing(&self) -> Arc<Notify> {
        let gate = Arc::new(Notify::new());
        *self.hold.lock().unwrap() = Some(Arc::clone(&gate));
        gate
    }

    /// Fail the listing [`Self::hold_next_listing`] holds once it is let go.
    pub(crate) fn fail_the_held_listing(&self) {
        *self.fail_held.lock().unwrap() = true;
    }

    /// Wait until a listing reached the gate [`Self::hold_next_listing`] set.
    pub(crate) async fn listing_held(&self) {
        timeout(Duration::from_secs(10), self.held.notified())
            .await
            .expect("no listing reached the hold");
    }
}

/// `GET /athlete`: the profile, with its `created_at` when the mock has one.
async fn athlete_profile(State(mock): State<Arc<MockStrava>>) -> Json<Value> {
    *mock.profiles.lock().unwrap() += 1;
    let created = *mock.created_at.lock().unwrap();
    let mut profile = json!({ "id": 4242, "username": "walker", "firstname": "Wal" });
    if let Some(created) = created {
        profile["created_at"] = json!(Utc.timestamp_opt(created, 0).unwrap().to_rfc3339());
    }
    Json(profile)
}

/// `GET /athlete/activities`: the activities within `after`/`before`, newest
/// first, at most `per_page` of them.
async fn list_activities(
    State(mock): State<Arc<MockStrava>>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, StatusCode> {
    let bound = |name: &str| query.get(name).and_then(|value| value.parse::<i64>().ok());
    let (after, before) = (bound("after"), bound("before"));
    mock.listings.lock().unwrap().push((after, before));
    let hold = mock.hold.lock().unwrap().take();
    if let Some(gate) = hold {
        mock.held.notify_one();
        gate.notified().await;
        if mem::take(&mut *mock.fail_held.lock().unwrap()) {
            return Err(StatusCode::BAD_REQUEST);
        }
    }
    let per_page: usize = query
        .get("per_page")
        .and_then(|value| value.parse().ok())
        .unwrap_or(30);
    mock.page_sizes.lock().unwrap().push(per_page);
    let mut listed: Vec<i64> = mock
        .starts
        .lock()
        .unwrap()
        .iter()
        .copied()
        .filter(|start| after.is_none_or(|after| *start > after))
        .filter(|start| before.is_none_or(|before| *start < before))
        .collect();
    listed.sort_unstable_by(|a, b| b.cmp(a));
    listed.truncate(per_page);
    Ok(Json(Value::Array(
        listed.iter().map(|start| ride_json(*start)).collect(),
    )))
}

/// `GET /activities/{id}`: the ride whose id is its start, as a detail.
async fn activity_detail(State(mock): State<Arc<MockStrava>>, Path(id): Path<i64>) -> Json<Value> {
    mock.details.lock().unwrap().push(id);
    let mut detail = ride_json(id);
    detail["description"] = json!("Sortie au parc");
    Json(detail)
}

/// The stand-in's ride starting at `start`, whose id is its start.
fn ride_json(start: i64) -> Value {
    json!({
        "id": start,
        "name": "Morning ride",
        "type": "Ride",
        "sport_type": "Ride",
        "start_date": Utc.timestamp_opt(start, 0).unwrap().to_rfc3339(),
        "distance": 20_000.0,
        "elapsed_time": 3_600,
        "total_elevation_gain": 10.0
    })
}

pub(crate) async fn mock_strava() -> (String, Arc<MockStrava>) {
    let mock = Arc::new(MockStrava::default());
    let app = Router::new()
        .route("/athlete", get(athlete_profile))
        .route("/athlete/activities", get(list_activities))
        .route("/activities/{id}", get(activity_detail))
        .with_state(Arc::clone(&mock));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), mock)
}

/// A server whose Strava provider talks to `api_base`, with a window of
/// `per_window` requests a day, a quarter of which background work takes.
pub(crate) async fn server(api_base: &str, per_window: u32) -> Arc<ServerContext> {
    let mut config = common::test_server_config_with_api_base("strava", api_base);
    common::set_server_app(&mut config, "strava", "walk_client", "walk_secret");
    let resources = common::create_test_server_resources_with_config(config)
        .await
        .unwrap();
    with_window(&resources, per_window)
}

/// `resources` with a Strava window of `per_window` requests a day, reaching
/// the scraper service its config names.
pub(crate) fn with_window(resources: &Arc<ServerContext>, per_window: u32) -> Arc<ServerContext> {
    let limiter = ProviderRateLimiter::new(
        Arc::clone(&resources.common.repos.usage_counters),
        Arc::clone(&resources.common.repos.provider_connections),
    );
    limiter.set_budgets("strava", &[(per_window, ONE_DAY)]);
    let mut context = (**resources).clone();
    let sciotte = &resources.common.config.external_services.sciotte;
    context.fitness.provider_registry = Arc::new(
        ProviderRegistry::new()
            .with_endpoint_overrides(&resources.common.config.oauth.endpoints)
            .with_request_limiter(Arc::new(limiter))
            .with_sciotte_client(startup_client(
                sciotte.remote_url.as_deref(),
                sciotte.audience.as_deref(),
            )),
    );
    Arc::new(context)
}

/// Open a fresh Strava window: the windows are counted in the database.
pub(crate) async fn open_window(resources: &Arc<ServerContext>) {
    resources
        .common
        .repos
        .usage_counters
        .delete_old_counters("9999")
        .await
        .unwrap();
}

/// Wait until no step holds one of the athlete's `kind` job rows: a row is
/// leased from its record, and from each claim, until its step settles it.
///
/// A sweep run by a test opens a fresh window ([`open_window`]) only then:
/// a window opened under a step part-way through its day would let that
/// step read past the day it was refused in, which no real day's end does.
pub(crate) async fn steps_settled(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    kind: BackfillJobKind,
    deadline: Instant,
) {
    let sql = "SELECT COUNT(*) AS held FROM activity_backfill_jobs \
               WHERE user_id = $1 AND kind = $2 AND leased_until_ms > $3";
    macro_rules! held {
        ($pool:expr) => {{
            sqlx::query(sql)
                .bind(user_id.to_string())
                .bind(kind.as_str())
                .bind(Utc::now().timestamp_millis())
                .fetch_one($pool)
                .await
                .unwrap()
                .get::<i64, _>("held")
        }};
    }
    loop {
        let held = match resources.agent.database.backend() {
            DatabaseBackend::SQLite(sqlite) => held!(sqlite.pool()),
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(pg) => held!(pg.pool()),
        };
        if held == 0 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "a step still held the athlete's {} row at the deadline",
            kind.as_str()
        );
        sleep(Duration::from_millis(20)).await;
    }
}

/// An athlete holding a live Strava grant and its connection.
pub(crate) async fn strava_athlete(resources: &Arc<ServerContext>) -> (Uuid, TenantId) {
    let (user_id, _user, tenant) = common::create_test_user_with_plan(
        &resources.agent.database,
        &format!("walk-{}@example.com", Uuid::new_v4()),
        "starter",
    )
    .await
    .unwrap();
    hold_strava_grant(resources, user_id, tenant).await;
    (user_id, tenant)
}

/// Give the athlete a live Strava grant and its connection.
pub(crate) async fn hold_strava_grant(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) {
    let now = Utc::now();
    let repos = &resources.common.repos;
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id,
            tenant_id: tenant.to_string(),
            provider: oauth_providers::STRAVA.to_owned(),
            // >= 40 chars and not "at_"-prefixed: the provider's token validation.
            access_token: SecretString::from(format!("strava_access_token_{:0>30}", 7)),
            refresh_token: Some(SecretString::from("strava_refresh")),
            token_type: "Bearer".to_owned(),
            expires_at: Some(now + ChronoDuration::hours(6)),
            scope: Some("activity:read_all".to_owned()),
            provider_user_id: Some("4242".to_owned()),
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    repos
        .provider_connections
        .register_connection(
            user_id,
            tenant,
            oauth_providers::STRAVA,
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
}

/// What the ledger holds for the athlete's walk of `backend`.
async fn walk_state(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    backend: &str,
) -> Option<BackfillJobState> {
    resources
        .common
        .repos
        .activity_backfill_jobs
        .backfill_job_state(user_id, tenant, backend, BackfillJobKind::HistoryWalk)
        .await
        .unwrap()
}

/// The walk's progress on its row.
async fn progress(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId) -> JobProgress {
    resources
        .common
        .repos
        .activity_backfill_jobs
        .job_progress(
            user_id,
            tenant,
            oauth_providers::STRAVA,
            BackfillJobKind::HistoryWalk,
        )
        .await
        .unwrap()
        .expect("the walk's row is kept")
}

/// The walk's row as a step that released it left it.
struct ReleasedRow {
    attempts: i64,
    before_ts: Option<i64>,
    next_attempt_ms: i64,
}

const DAY_MS: i64 = DAY * 1_000;

/// A paused step's retry is the end of the day window that refused it. The
/// refusal floors the wait to whole seconds, and the step adds it to its own
/// later clock, so the instant lands within a few seconds of the end. The
/// refusal happened between `asked_from_ms`, before the step was started,
/// and `released_by_ms`, once its row was seen released: its window is the
/// day of one of the two, never the day the assert runs in, so a run that
/// crosses 00:00 UTC still reads the window that refused it, and the retry
/// lands near one of those two ends, never anywhere between them.
fn assert_waits_for_the_window_end(next_attempt_ms: i64, asked_from_ms: i64, released_by_ms: i64) {
    let window_end = |at_ms: i64| (at_ms / DAY_MS + 1) * DAY_MS;
    let ends = [window_end(asked_from_ms), window_end(released_by_ms)];
    assert!(
        ends.iter()
            .any(|end| (end - 2_000..=end + 5_000).contains(&next_attempt_ms)),
        "bug: a spent day's walk waits for the end of the window that refused it \
         ({ends:?}), not a lease: {next_attempt_ms}"
    );
}

/// The athlete's walk row once no step holds its lease; `None` while one
/// does.
async fn released_row(resources: &Arc<ServerContext>, user_id: Uuid) -> Option<ReleasedRow> {
    let sql = "SELECT attempts, before_ts, next_attempt_ms FROM activity_backfill_jobs \
               WHERE user_id = $1 AND leased_until_ms = 0";
    macro_rules! read {
        ($pool:expr) => {{
            let row = sqlx::query(sql)
                .bind(user_id.to_string())
                .fetch_optional($pool)
                .await
                .unwrap()?;
            Some(ReleasedRow {
                attempts: row.get("attempts"),
                before_ts: row.get("before_ts"),
                next_attempt_ms: row.get("next_attempt_ms"),
            })
        }};
    }
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => read!(sqlite.pool()),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => read!(pg.pool()),
    }
}

/// Run the walk the sweep's way, one step a pass, in a fresh window each
/// pass, until its row ends.
async fn walk_ended(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId) {
    let deadline = Instant::now() + WALK_DEADLINE;
    loop {
        steps_settled(resources, user_id, BackfillJobKind::HistoryWalk, deadline).await;
        let state = walk_state(resources, user_id, tenant, oauth_providers::STRAVA).await;
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
        open_window(resources).await;
        paused_retries_due(resources, user_id).await;
        age_job_rows(resources, user_id).await;
        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        resume_backfill_jobs(runtime).await.unwrap();
        sleep(Duration::from_millis(50)).await;
    }
}

/// The athlete's activities stored under `key`.
pub(crate) async fn rows_under(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    key: &str,
) -> i64 {
    resources
        .common
        .repos
        .activity_cache
        .count_by_provider(user_id, &tenant)
        .await
        .unwrap()
        .into_iter()
        .find(|(provider, _)| provider == key)
        .map_or(0, |(_, count)| count)
}

/// A Strava grant alone is walked through the API, a year a window, newest
/// first. The first step reads the profile for the account's creation, the
/// walk's floor, which gives the walk its length; the walk ends there, every
/// activity in, the row kept as walked.
#[tokio::test]
async fn a_strava_grant_is_walked_a_year_a_window_down_to_the_account_creation() {
    let (api_base, mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    let created = now - 1_000 * DAY;
    *mock.starts.lock().unwrap() = vec![now - 10 * DAY, now - 400 * DAY, now - 800 * DAY];
    *mock.created_at.lock().unwrap() = Some(created);
    open_window(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert!(
        start_history_walk(
            &runtime,
            user_id,
            tenant,
            oauth_providers::STRAVA,
            WalkTrigger::DepthMissing
        )
        .await,
        "bug: a live grant is walked"
    );
    walk_ended(&resources, user_id, tenant).await;

    assert_eq!(
        walk_state(&resources, user_id, tenant, oauth_providers::STRAVA).await,
        Some(BackfillJobState::Walked),
        "bug: the walk reaches the account's creation"
    );
    assert_eq!(
        rows_under(&resources, user_id, tenant, oauth_providers::STRAVA).await,
        3,
        "bug: every activity of the account lands under the grant's key"
    );
    assert_eq!(
        *mock.profiles.lock().unwrap(),
        1,
        "bug: the profile is read once, on the first step"
    );
    let listings = mock.listings.lock().unwrap().clone();
    assert_eq!(
        listings.len(),
        3,
        "bug: one window a step, each landing one ride: {listings:?}"
    );
    for (after, before) in &listings {
        let (after, before) = (
            after.expect("a window's floor"),
            before.expect("a window's top"),
        );
        assert!(
            before - after <= 366 * DAY,
            "bug: an API window spans a year at most: {listings:?}"
        );
        assert!(
            after >= created,
            "bug: no window reads below the account's creation: {listings:?}"
        );
    }
    assert_eq!(
        listings.last().and_then(|(after, _)| *after),
        Some(created),
        "bug: the last window stops at the account's creation"
    );
    let walked = progress(&resources, user_id, tenant).await;
    assert_eq!(
        walked.units_total,
        Some((now - created) / 2_629_746),
        "bug: the account's creation gives the walk its length"
    );
    assert_eq!(walked.rows_landed, 3, "bug: the progress counts the rows");
    assert!(
        walked.units_done >= 32,
        "bug: the walk covered the account's months: {walked:?}"
    );
}

/// A spent request budget pauses the walk: the claim goes back with its
/// attempt and its cursor, nothing is lost or marked incomplete, and the
/// sweep resumes the walk in the next window from where it stopped.
#[tokio::test]
async fn a_spent_budget_pauses_the_walk_and_the_next_window_resumes_it() {
    let (api_base, mock) = mock_strava().await;
    // Eight requests a window, two of them background: the profile and the
    // first window, then nothing until the next window.
    let resources = server(&api_base, 8).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    *mock.starts.lock().unwrap() = vec![now - 10 * DAY, now - 400 * DAY];
    *mock.created_at.lock().unwrap() = Some(now - 600 * DAY);
    open_window(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert!(
        start_history_walk(
            &runtime,
            user_id,
            tenant,
            oauth_providers::STRAVA,
            WalkTrigger::DepthMissing
        )
        .await,
        "bug: a live grant is walked"
    );
    let deadline = Instant::now() + WALK_DEADLINE;
    while progress(&resources, user_id, tenant).await.rows_landed == 0 {
        assert!(Instant::now() < deadline, "the first step never landed");
        sleep(Duration::from_millis(50)).await;
    }
    // Same window: the walk's share is spent, so the next step pauses.
    age_job_rows(&resources, user_id).await;
    let asked_from_ms = Utc::now().timestamp_millis();
    assert_eq!(
        resume_backfill_jobs(Arc::clone(&runtime)).await.unwrap(),
        1,
        "bug: the sweep takes the walk's next step"
    );
    let paused_at = mock.listings.lock().unwrap().len();
    let deadline = Instant::now() + WALK_DEADLINE;
    let paused = loop {
        if let Some(paused) = released_row(&resources, user_id).await {
            break paused;
        }
        assert!(
            Instant::now() < deadline,
            "the paused step never released its row"
        );
        sleep(Duration::from_millis(50)).await;
    };
    let released_by_ms = Utc::now().timestamp_millis();
    assert_eq!(
        paused.attempts, 0,
        "bug: a paused step hands its attempt back"
    );
    assert!(
        paused
            .before_ts
            .is_some_and(|before| before < now - 300 * DAY),
        "bug: the paused walk keeps the cursor its first step reached: {:?}",
        paused.before_ts
    );
    assert_waits_for_the_window_end(paused.next_attempt_ms, asked_from_ms, released_by_ms);
    assert!(
        resources
            .common
            .repos
            .activity_backfill_jobs
            .claim_stale_backfill_jobs(BackfillJobClaim {
                // The last instant before its retry, never the assert's own
                // clock, which can already be past the window's end.
                now_ms: paused.next_attempt_ms - 1,
                queued_older_than_ms: 0,
                lease_ms: 0,
                max_attempts: 10,
                limit: 10,
            })
            .await
            .unwrap()
            .is_empty(),
        "bug: a walk paused on a spent day is claimed again before the day ends"
    );
    assert_eq!(
        paused_at, 1,
        "bug: a spent budget sends no listing: only the first step's went out"
    );
    assert_eq!(
        walk_state(&resources, user_id, tenant, oauth_providers::STRAVA).await,
        Some(BackfillJobState::Owed),
        "bug: a paused walk stays owed"
    );

    walk_ended(&resources, user_id, tenant).await;
    assert_eq!(
        walk_state(&resources, user_id, tenant, oauth_providers::STRAVA).await,
        Some(BackfillJobState::Walked),
        "bug: the next window resumes the walk to its end"
    );
    assert_eq!(
        rows_under(&resources, user_id, tenant, oauth_providers::STRAVA).await,
        2,
        "bug: the resumed walk lands the rest of the history"
    );
}

/// A budget spent before the walk's first read, its profile's, pauses the
/// walk until the window ends as a spent listing does: nothing is sent, the
/// attempt is handed back, and a window with room resumes it from the top.
#[tokio::test]
async fn a_spent_budget_on_the_profile_read_pauses_the_walk_until_the_window_ends() {
    let (api_base, mock) = mock_strava().await;
    // Three requests a window leave the background share none at all.
    let resources = server(&api_base, 3).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    *mock.starts.lock().unwrap() = vec![now - 10 * DAY];
    *mock.created_at.lock().unwrap() = Some(now - 100 * DAY);
    open_window(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let asked_from_ms = Utc::now().timestamp_millis();
    assert!(
        start_history_walk(
            &runtime,
            user_id,
            tenant,
            oauth_providers::STRAVA,
            WalkTrigger::DepthMissing
        )
        .await,
        "bug: a live grant is walked"
    );
    let deadline = Instant::now() + WALK_DEADLINE;
    let paused = loop {
        if let Some(paused) = released_row(&resources, user_id).await {
            break paused;
        }
        assert!(
            Instant::now() < deadline,
            "the paused step never released its row"
        );
        sleep(Duration::from_millis(50)).await;
    };
    let released_by_ms = Utc::now().timestamp_millis();
    assert_eq!(
        *mock.profiles.lock().unwrap(),
        0,
        "bug: a spent budget sends the profile read"
    );
    assert_eq!(
        paused.attempts, 0,
        "bug: a paused step hands its attempt back"
    );
    assert_waits_for_the_window_end(paused.next_attempt_ms, asked_from_ms, released_by_ms);

    let roomy = with_window(&resources, 400);
    walk_ended(&roomy, user_id, tenant).await;
    assert_eq!(
        walk_state(&roomy, user_id, tenant, oauth_providers::STRAVA).await,
        Some(BackfillJobState::Walked),
        "bug: a window with room resumes the paused walk to its end"
    );
    assert_eq!(
        *mock.profiles.lock().unwrap(),
        1,
        "bug: the resumed walk reads the profile it was refused"
    );
}

/// With no creation date on the profile, an API walk ends after two
/// consecutive empty years, never at the first one: a year off training is
/// no end of the history.
#[tokio::test]
async fn an_api_walk_with_no_floor_ends_after_two_empty_years() {
    let (api_base, mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    // A ride this month, one three years back past a one-year gap, then
    // nothing for two years.
    *mock.starts.lock().unwrap() = vec![now - 20 * DAY, now - 3 * 365 * DAY];
    open_window(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert!(
        start_history_walk(
            &runtime,
            user_id,
            tenant,
            oauth_providers::STRAVA,
            WalkTrigger::DepthMissing
        )
        .await,
        "bug: a live grant is walked"
    );
    walk_ended(&resources, user_id, tenant).await;

    assert_eq!(
        rows_under(&resources, user_id, tenant, oauth_providers::STRAVA).await,
        2,
        "bug: one empty year never ends the walk: the ride past it lands"
    );
    let walked = progress(&resources, user_id, tenant).await;
    assert_eq!(
        walked.units_total, None,
        "bug: a profile with no creation date leaves the walk's length unknown"
    );
    let listings = mock.listings.lock().unwrap().clone();
    let last_two: Vec<_> = listings.iter().rev().take(2).collect();
    assert!(
        last_two.iter().all(|(after, before)| mock
            .starts
            .lock()
            .unwrap()
            .iter()
            .all(|start| !(after.is_none_or(|a| *start > a) && before.is_none_or(|b| *start < b)))),
        "bug: the walk ends on two empty windows: {listings:?}"
    );
    assert!(
        listings
            .iter()
            .all(|(after, _)| after.is_some_and(|after| after > now - 6 * 365 * DAY)),
        "bug: the walk stops two empty years below its last ride: {listings:?}"
    );
}

/// A depth ask never walks a source whose walk already reached its end: it
/// holds all there is.
#[tokio::test]
async fn a_depth_ask_never_rewalks_a_walked_source() {
    let (api_base, mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    *mock.starts.lock().unwrap() = vec![now - 10 * DAY];
    *mock.created_at.lock().unwrap() = Some(now - 200 * DAY);
    open_window(&resources).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let walk = || {
        start_history_walk(
            &runtime,
            user_id,
            tenant,
            oauth_providers::STRAVA,
            WalkTrigger::DepthMissing,
        )
    };

    assert!(walk().await, "bug: a depth ask walks a source never walked");
    walk_ended(&resources, user_id, tenant).await;
    assert_eq!(
        walk_state(&resources, user_id, tenant, oauth_providers::STRAVA).await,
        Some(BackfillJobState::Walked),
        "the walk reached its end"
    );
    let listed = mock.listings.lock().unwrap().len();

    assert!(
        !walk().await,
        "bug: a depth ask re-walked a source whose walk already ended"
    );
    assert_eq!(
        mock.listings.lock().unwrap().len(),
        listed,
        "bug: a refused ask lists nothing"
    );
}

/// An API step asks Strava's largest page, and a page that comes back full
/// reads the next page below its oldest activity: a busy year is never cut
/// at the first two hundred rides.
#[tokio::test]
async fn a_full_api_page_reads_the_next_below_its_oldest_activity() {
    let (api_base, mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    // 250 rides in the last year, one a day: more than one page.
    let starts: Vec<i64> = (1..=250).map(|day| now - day * DAY).collect();
    *mock.starts.lock().unwrap() = starts.clone();
    *mock.created_at.lock().unwrap() = Some(now - 300 * DAY);
    open_window(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert!(
        start_history_walk(
            &runtime,
            user_id,
            tenant,
            oauth_providers::STRAVA,
            WalkTrigger::DepthMissing
        )
        .await,
        "bug: a live grant is walked"
    );
    walk_ended(&resources, user_id, tenant).await;

    let page_sizes = mock.page_sizes.lock().unwrap().clone();
    assert!(
        page_sizes.iter().all(|size| *size == 200),
        "bug: every API page asks Strava's largest page: {page_sizes:?}"
    );
    let listings = mock.listings.lock().unwrap().clone();
    let mut newest_first = starts;
    newest_first.sort_unstable_by(|a, b| b.cmp(a));
    let oldest_of_first_page = newest_first[199];
    assert_eq!(
        listings.get(1).and_then(|(_, before)| *before),
        Some(oldest_of_first_page),
        "bug: the page after a full one reads below its oldest activity: {listings:?}"
    );
    assert_eq!(
        rows_under(&resources, user_id, tenant, oauth_providers::STRAVA).await,
        250,
        "bug: every ride of a year past one page lands"
    );
}
