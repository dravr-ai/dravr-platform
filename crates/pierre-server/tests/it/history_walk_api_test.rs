// ABOUTME: A history walk through an API grant: a year a window, the Strava account's creation as its floor, a budget pause
// ABOUTME: And the one source order: a live session before the grant, never the push, and a finished walk never re-asked by depth

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
// after two empty years. The source order puts a live session before the
// grant, and a depth ask never walks a source whose walk already ended.
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
use crate::helpers::sciotte_mock::seed_scrape_session;
use crate::history_walk_test::age_job_rows;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{Duration as ChronoDuration, TimeZone, Utc};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{ConnectionType, TenantId, UserOAuthToken};
use pierre_database::repositories::{
    BackfillJobClaim, BackfillJobKind, BackfillJobState, JobProgress,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::request_budget::{ProviderRateLimiter, ONE_DAY};
use pierre_services::training_history_read::{
    history_source_order, walk_source, HistorySource, HistorySourceRole,
};
use pierre_tool_runtime::activity_backfill::history_walk::{start_history_walk, WalkTrigger};
use pierre_tool_runtime::activity_backfill_resume::resume_backfill_jobs;
use pierre_tool_runtime::runtime::ToolRuntime;
use secrecy::SecretString;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::time::sleep;
use uuid::Uuid;

/// Seconds in a day.
const DAY: i64 = 86_400;

/// How long a walk gets to end before the test gives up.
const WALK_DEADLINE: Duration = Duration::from_secs(20);

/// What the Strava stand-in serves and saw.
#[derive(Default)]
struct MockStrava {
    /// Activity starts (unix seconds) the athlete's history holds.
    starts: Mutex<Vec<i64>>,
    /// When the account was created, if the profile says.
    created_at: Mutex<Option<i64>>,
    /// Every listing's `after` and `before`.
    listings: Mutex<Vec<(Option<i64>, Option<i64>)>>,
    /// Profile reads.
    profiles: Mutex<usize>,
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
) -> Json<Value> {
    let bound = |name: &str| query.get(name).and_then(|value| value.parse::<i64>().ok());
    let (after, before) = (bound("after"), bound("before"));
    mock.listings.lock().unwrap().push((after, before));
    let per_page: usize = query
        .get("per_page")
        .and_then(|value| value.parse().ok())
        .unwrap_or(30);
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
    Json(Value::Array(
        listed
            .iter()
            .map(|start| {
                json!({
                    "id": start,
                    "name": "Morning ride",
                    "type": "Ride",
                    "sport_type": "Ride",
                    "start_date": Utc.timestamp_opt(*start, 0).unwrap().to_rfc3339(),
                    "distance": 20_000.0,
                    "elapsed_time": 3_600,
                    "total_elevation_gain": 10.0
                })
            })
            .collect(),
    ))
}

async fn mock_strava() -> (String, Arc<MockStrava>) {
    let mock = Arc::new(MockStrava::default());
    let app = Router::new()
        .route("/athlete", get(athlete_profile))
        .route("/athlete/activities", get(list_activities))
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
async fn server(api_base: &str, per_window: u32) -> Arc<ServerContext> {
    let mut config = common::test_server_config_with_api_base("strava", api_base);
    common::set_server_app(&mut config, "strava", "walk_client", "walk_secret");
    let resources = common::create_test_server_resources_with_config(config)
        .await
        .unwrap();
    with_window(&resources, per_window)
}

/// `resources` with a Strava window of `per_window` requests a day.
fn with_window(resources: &Arc<ServerContext>, per_window: u32) -> Arc<ServerContext> {
    let limiter = ProviderRateLimiter::new(
        Arc::clone(&resources.common.repos.usage_counters),
        Arc::clone(&resources.common.repos.provider_connections),
    );
    limiter.set_budgets("strava", &[(per_window, ONE_DAY)]);
    let mut context = (**resources).clone();
    context.fitness.provider_registry = Arc::new(
        ProviderRegistry::new()
            .with_endpoint_overrides(&resources.common.config.oauth.endpoints)
            .with_request_limiter(Arc::new(limiter)),
    );
    Arc::new(context)
}

/// Open a fresh Strava window: the windows are counted in the database.
async fn open_window(resources: &Arc<ServerContext>) {
    resources
        .common
        .repos
        .usage_counters
        .delete_old_counters("9999")
        .await
        .unwrap();
}

/// An athlete holding a live Strava grant and its connection.
async fn strava_athlete(resources: &Arc<ServerContext>) -> (Uuid, TenantId) {
    let (user_id, _user, tenant) = common::create_test_user_with_plan(
        &resources.agent.database,
        &format!("walk-{}@example.com", Uuid::new_v4()),
        "starter",
    )
    .await
    .unwrap();
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
    (user_id, tenant)
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

/// Run the walk the sweep's way, one step a pass, in a fresh window each
/// pass, until its row ends.
async fn walk_ended(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId) {
    let deadline = Instant::now() + WALK_DEADLINE;
    loop {
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
        age_job_rows(resources, user_id).await;
        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        resume_backfill_jobs(runtime).await.unwrap();
        sleep(Duration::from_millis(50)).await;
    }
}

/// The athlete's activities stored under `key`.
async fn rows_under(
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
            WalkTrigger::Connected
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
        walked.months_total,
        Some((now - created) / 2_629_746),
        "bug: the account's creation gives the walk its length"
    );
    assert_eq!(walked.rows_landed, 3, "bug: the progress counts the rows");
    assert!(
        walked.months_done >= 32,
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
            WalkTrigger::Connected
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
    assert_eq!(
        resume_backfill_jobs(Arc::clone(&runtime)).await.unwrap(),
        1,
        "bug: the sweep takes the walk's next step"
    );
    let paused_at = mock.listings.lock().unwrap().len();
    let deadline = Instant::now() + WALK_DEADLINE;
    loop {
        let claimed = resources
            .common
            .repos
            .activity_backfill_jobs
            .claim_stale_backfill_jobs(BackfillJobClaim {
                now_ms: Utc::now().timestamp_millis(),
                queued_older_than_ms: 0,
                lease_ms: 0,
                max_attempts: 10,
                limit: 10,
            })
            .await
            .unwrap();
        if let Some(paused) = claimed.into_iter().next() {
            assert_eq!(
                paused.attempts, 1,
                "bug: a paused step hands its attempt back"
            );
            assert!(
                paused
                    .before_ts
                    .is_some_and(|before| before < now - 300 * DAY),
                "bug: the paused walk keeps the cursor its first step reached: {:?}",
                paused.before_ts
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the paused step never released its row"
        );
        sleep(Duration::from_millis(50)).await;
    }
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
            WalkTrigger::Connected
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
        walked.months_total, None,
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

/// The one source order: a live usr/pwd session before the grant, so a walk
/// reads the session; a flagged session is passed over for the grant; a
/// Garmin athlete holding only the push has nothing to walk.
#[tokio::test]
async fn a_walk_reads_a_live_session_before_the_grant_and_never_the_push() {
    let (api_base, _mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let repos = &resources.common.repos;

    assert_eq!(
        walk_source(repos, user_id, tenant, oauth_providers::STRAVA)
            .await
            .map(|source| source.backend),
        Some(oauth_providers::STRAVA.to_owned()),
        "bug: with no session, the grant is walked"
    );
    seed_scrape_session(
        &resources,
        user_id,
        tenant,
        oauth_providers::SCIOTTE,
        "walk-session",
    )
    .await;
    repos
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
    assert_eq!(
        history_source_order(repos, user_id, tenant, oauth_providers::STRAVA).await,
        vec![
            HistorySource {
                backend: oauth_providers::SCIOTTE.to_owned(),
                role: HistorySourceRole::Session,
                live: true,
            },
            HistorySource {
                backend: oauth_providers::STRAVA.to_owned(),
                role: HistorySourceRole::Grant,
                live: true,
            },
        ],
        "bug: the session comes before the grant"
    );
    assert_eq!(
        walk_source(repos, user_id, tenant, oauth_providers::STRAVA)
            .await
            .map(|source| source.backend),
        Some(oauth_providers::SCIOTTE.to_owned()),
        "bug: a live session is walked before the grant (D2)"
    );
    repos
        .provider_connections
        .mark_needs_reauth(
            user_id,
            tenant,
            oauth_providers::SCIOTTE,
            Some("session_expired"),
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(
        walk_source(repos, user_id, tenant, oauth_providers::STRAVA)
            .await
            .map(|source| source.backend),
        Some(oauth_providers::STRAVA.to_owned()),
        "bug: a flagged session is passed over for the grant"
    );

    let (garmin_user, _user, garmin_tenant) = common::create_test_user_with_plan(
        &resources.agent.database,
        &format!("push-{}@example.com", Uuid::new_v4()),
        "starter",
    )
    .await
    .unwrap();
    seed_scrape_session(
        &resources,
        garmin_user,
        garmin_tenant,
        oauth_providers::GARMIN,
        "push-grant",
    )
    .await;
    repos
        .provider_connections
        .register_connection(
            garmin_user,
            garmin_tenant,
            oauth_providers::GARMIN,
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        walk_source(repos, garmin_user, garmin_tenant, oauth_providers::GARMIN).await,
        None,
        "bug: a push holds nothing from before the grant, so nothing is walked"
    );
}

/// A depth ask never walks a source whose walk already reached its end: it
/// holds all there is. A new connection walks it again.
#[tokio::test]
async fn a_depth_ask_never_rewalks_a_walked_source_but_a_connection_does() {
    let (api_base, mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    *mock.starts.lock().unwrap() = vec![now - 10 * DAY];
    *mock.created_at.lock().unwrap() = Some(now - 200 * DAY);
    open_window(&resources).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let walk =
        |trigger| start_history_walk(&runtime, user_id, tenant, oauth_providers::STRAVA, trigger);

    assert!(
        walk(WalkTrigger::DepthMissing).await,
        "bug: a depth ask walks a source never walked"
    );
    walk_ended(&resources, user_id, tenant).await;
    assert_eq!(
        walk_state(&resources, user_id, tenant, oauth_providers::STRAVA).await,
        Some(BackfillJobState::Walked),
        "the walk reached its end"
    );

    assert!(
        !walk(WalkTrigger::DepthMissing).await,
        "bug: a depth ask re-walked a source whose walk already ended"
    );
    assert!(
        walk(WalkTrigger::Connected).await,
        "bug: a new connection walks the source again"
    );
    assert_eq!(
        walk_state(&resources, user_id, tenant, oauth_providers::STRAVA).await,
        Some(BackfillJobState::Owed),
        "bug: the new walk replaces the ended row"
    );
}
