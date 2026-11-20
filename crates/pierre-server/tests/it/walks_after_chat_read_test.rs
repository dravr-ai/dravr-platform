// ABOUTME: A chat turn whose live read of a sciotte session carried the list head asks the session's walks
// ABOUTME: Its incomplete history walk resumes from its cursor past the back-off; a head-less, grant-served or grant read asks nothing

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The walks a chat read asks (carnet#1679, carnet#1698).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// The capture sweep asks a session's tag walk and track walk once its head
// refresh succeeds. A session the sweep passes over was never asked, so a
// chat turn's own live read of the session's head asks them too. Before
// them it resumes, from its cursor, the session's history walk that ended
// incomplete, once its failing end is a day old: nothing else asks such a
// walk again on its own. A grant is no session the walks count, so its read
// resumes nothing. The scraper and Strava's API are one loopback stand-in
// (a test double, per the repo's mock rule) that counts the list reads each
// serves.
#![cfg(all(
    feature = "provider-strava",
    feature = "provider-sciotte",
    feature = "provider-garmin",
    feature = "tools-data",
    feature = "health-sync"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::detail_walk_edges_test::{accept_walk_rows, refuse_walk_rows_on};
use crate::helpers::strava_grant::hold_strava_grant;
use crate::history_walk_api_test::with_window;
use crate::history_walk_resume_test::update_walk_row;
use crate::history_walk_test::job_row;
use crate::track_walk_test::session_athlete;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{Duration as ChronoDuration, Utc};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_config::environment::ProviderEndpointOverride;
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::TenantId;
use pierre_database::repositories::{
    ActivityBackfillJobRow, BackfillJobKind, BackfillJobState, DetailWalkRow,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::activity_backfill::detail_walk::{self, TrackWalkAsked};
use pierre_tool_runtime::activity_backfill::tag_walk::{self, TagWalkAsked};
use pierre_tool_runtime::implementations::data::GetActivitiesTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::time;
use uuid::Uuid;

/// The key the athlete's session files its rides under.
const SESSION: &str = "sciotte";

/// The ride the session lists.
const SESSION_RIDE: &str = "15551234567";

/// The ride the grant lists.
const GRANT_RIDE: &str = "4001";

/// How long a detached ask gets to record its walks.
const ASKED_DEADLINE: Duration = Duration::from_secs(30);

/// How long a read that must ask nothing is given to ask anyway before the
/// test reads that it did not.
const SETTLE: Duration = Duration::from_secs(2);

/// The walks a head refresh asks for.
const WALKS: [BackfillJobKind; 2] = [BackfillJobKind::TagWalk, BackfillJobKind::TrackWalk];

/// What the stand-in serves and saw.
struct StandIn {
    /// Whether the scraper's capture reached the list head.
    head_complete: AtomicBool,
    /// The session's list reads.
    session_lists: AtomicUsize,
    /// The grant's list reads.
    grant_lists: AtomicUsize,
}

async fn spawn_stand_in() -> (String, Arc<StandIn>) {
    let stand_in = Arc::new(StandIn {
        head_complete: AtomicBool::new(true),
        session_lists: AtomicUsize::new(0),
        grant_lists: AtomicUsize::new(0),
    });
    let app = Router::new()
        .route(
            "/auth/import-session",
            post(|| async { Json(json!({ "session_id": "cap-verified-session" })) }),
        )
        .route(
            "/api/athlete",
            get(|| async { Json(json!({ "display_name": "Cap Tester" })) }),
        )
        .route(
            "/api/activities",
            get(|State(stand_in): State<Arc<StandIn>>| async move {
                stand_in.session_lists.fetch_add(1, Ordering::SeqCst);
                Json(json!({
                    "count": 1,
                    "activities": [session_ride()],
                    "head_complete": stand_in.head_complete.load(Ordering::SeqCst)
                }))
            }),
        )
        .route(
            "/api/activity-search",
            get(|| async { Json(json!({ "count": 0, "activities": [], "head_complete": true })) }),
        )
        .route(
            "/athlete/activities",
            get(|State(stand_in): State<Arc<StandIn>>| async move {
                stand_in.grant_lists.fetch_add(1, Ordering::SeqCst);
                (StatusCode::OK, Json(json!([grant_ride()])))
            }),
        )
        .with_state(Arc::clone(&stand_in));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), stand_in)
}

fn session_ride() -> Value {
    json!({
        "id": SESSION_RIDE,
        "name": "Sortie vélo matinale",
        "sport_type": "ride",
        "start_date": (Utc::now() - ChronoDuration::days(2)).to_rfc3339(),
        "duration_seconds": 2_700,
        "provider": "strava",
        "distance_meters": 21_000.0
    })
}

fn grant_ride() -> Value {
    json!({
        "id": 4001,
        "name": "Sortie longue",
        "type": "Ride",
        "sport_type": "Ride",
        "start_date": (Utc::now() - ChronoDuration::days(1)).to_rfc3339(),
        "distance": 80_000.0,
        "elapsed_time": 10_800,
        "moving_time": 10_200,
        "total_elevation_gain": 900.0
    })
}

/// A server reaching the stand-in as the scraper and as Strava's API, its
/// session walks counted in the walk windows of its Strava app.
async fn server(url: &str) -> Arc<ServerContext> {
    let mut config = common::test_server_config_with_sciotte(url);
    common::set_server_app(&mut config, "strava", "walk_client", "walk_secret");
    common::set_provider_endpoints(
        &mut config,
        "strava",
        ProviderEndpointOverride {
            api_base_url: Some(url.to_owned()),
            ..ProviderEndpointOverride::default()
        },
    );
    let resources = common::create_test_server_resources_with_config(config)
        .await
        .unwrap();
    with_window(&resources, 400)
}

/// An athlete holding only a Strava grant, in a tenant of their own.
async fn grant_athlete(resources: &Arc<ServerContext>) -> (Uuid, TenantId) {
    let (user_id, _user, tenant) = common::create_test_user_with_plan(
        &resources.agent.database,
        &format!("grant-only-{}@example.com", Uuid::new_v4()),
        "starter",
    )
    .await
    .unwrap();
    hold_strava_grant(resources, user_id, tenant).await;
    (user_id, tenant)
}

/// A chat turn's `get_activities` of `provider`'s recent window, `limit`
/// rows a page; the ids it served. Each turn of a test asks another `limit`,
/// so the response cache keyed on the window never answers it.
async fn turn(
    resources: &Arc<ServerContext>,
    athlete: (Uuid, TenantId),
    provider: &str,
    limit: u32,
) -> Vec<String> {
    let args = json!({ "provider": provider, "limit": limit, "mode": "summary" });
    turn_with(resources, athlete, args).await
}

/// A chat turn's `get_activities` with `args`; the ids it served.
async fn turn_with(
    resources: &Arc<ServerContext>,
    (user_id, tenant): (Uuid, TenantId),
    args: Value,
) -> Vec<String> {
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant.to_string())
        .with_auth_method("jwt_bearer");
    let response = GetActivitiesTool.execute(&runtime, &ctx, args).await;
    let payload = response
        .structured_content
        .expect("bug: get_activities answered no structured content");
    assert!(
        payload.get("error").is_none(),
        "bug: the turn's read failed: {payload}"
    );
    let served: Vec<String> = payload["activities"]
        .as_array()
        .expect("bug: the turn served no activity list")
        .iter()
        .filter_map(|activity| activity["id"].as_str().map(str::to_owned))
        .collect();
    served
}

/// When each of the walks a head refresh asks was recorded under `backend`
/// for the athlete (unix ms), or `None` for one never recorded.
async fn walks_recorded(
    resources: &Arc<ServerContext>,
    (user_id, tenant): (Uuid, TenantId),
    backend: &str,
) -> [Option<i64>; 2] {
    let ledger = &resources.common.repos.activity_backfill_jobs;
    let mut recorded = [None; 2];
    for (slot, kind) in recorded.iter_mut().zip(WALKS) {
        *slot = ledger
            .detail_walk_row(user_id, tenant, backend, kind)
            .await
            .unwrap()
            .map(|row| row.created_at_ms);
    }
    recorded
}

/// Wait until both walks are recorded under the session; when each was.
async fn walks_asked(resources: &Arc<ServerContext>, athlete: (Uuid, TenantId)) -> [i64; 2] {
    let deadline = Instant::now() + ASKED_DEADLINE;
    loop {
        if let [Some(tag), Some(track)] = walks_recorded(resources, athlete, SESSION).await {
            return [tag, track];
        }
        assert!(
            Instant::now() < deadline,
            "bug: a chat read that carried the session's list head did not ask its walks"
        );
        time::sleep(Duration::from_millis(50)).await;
    }
}

/// A chat read of the session's list head asks the session's tag walk and
/// track walk, once: the next read finds both standing and records neither
/// again (a direct ask answers `Standing`), and reads nothing more of the
/// provider than its own list. Another athlete's session, in another tenant,
/// is asked nothing.
#[tokio::test]
async fn a_chat_read_that_carried_the_list_head_asks_the_walks_once() {
    let (url, stand_in) = spawn_stand_in().await;
    let resources = server(&url).await;
    let athlete = session_athlete(&resources, "chat-walks").await;
    let other = session_athlete(&resources, "chat-walks-other").await;

    assert_eq!(
        turn(&resources, athlete, SESSION, 10).await,
        [SESSION_RIDE],
        "bug: the turn did not serve the session's ride"
    );
    let first = walks_asked(&resources, athlete).await;

    assert_eq!(
        turn(&resources, athlete, SESSION, 20).await,
        [SESSION_RIDE],
        "bug: the second turn did not serve the session's ride"
    );
    assert_eq!(
        stand_in.session_lists.load(Ordering::SeqCst),
        2,
        "bug: each turn did not read the session's list once, and only once"
    );
    time::sleep(SETTLE).await;
    assert_eq!(
        walks_recorded(&resources, athlete, SESSION).await,
        first.map(Some),
        "bug: the second turn recorded a walk the first had asked"
    );
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert_eq!(
        tag_walk::ask_tag_walk(&runtime, athlete.0, athlete.1, SESSION).await,
        TagWalkAsked::Standing,
        "bug: the tag walk the chat read asked is not standing"
    );
    assert_eq!(
        detail_walk::ask_track_walk(&runtime, athlete.0, athlete.1, SESSION).await,
        TrackWalkAsked::Standing,
        "bug: the track walk the chat read asked is not standing"
    );

    assert_eq!(
        walks_recorded(&resources, other, SESSION).await,
        [None, None],
        "bug: another athlete's session was asked walks it never read"
    );
    assert_eq!(
        walks_recorded(&resources, (athlete.0, other.1), SESSION).await,
        [None, None],
        "bug: the athlete's walks were recorded under another tenant"
    );
}

/// A capture that never reached the list head is served and asks nothing:
/// the head it did not read says nothing of a live session. The same
/// session's next read, which reaches it, asks both walks.
#[tokio::test]
async fn a_capture_without_the_list_head_asks_nothing() {
    let (url, stand_in) = spawn_stand_in().await;
    let resources = server(&url).await;
    let athlete = session_athlete(&resources, "chat-walks-headless").await;

    stand_in.head_complete.store(false, Ordering::SeqCst);
    assert_eq!(
        turn(&resources, athlete, SESSION, 10).await,
        [SESSION_RIDE],
        "bug: the head-less capture was not served"
    );
    time::sleep(SETTLE).await;
    assert_eq!(
        walks_recorded(&resources, athlete, SESSION).await,
        [None, None],
        "bug: a capture that never reached the list head asked the walks"
    );

    stand_in.head_complete.store(true, Ordering::SeqCst);
    assert_eq!(
        turn(&resources, athlete, SESSION, 20).await,
        [SESSION_RIDE],
        "bug: the complete capture was not served"
    );
    walks_asked(&resources, athlete).await;
}

/// A later page of the session's list starts below its head, so the read
/// says nothing of whether the head is current and asks nothing, though the
/// session answered it complete.
#[tokio::test]
async fn a_later_page_asks_nothing() {
    let (url, stand_in) = spawn_stand_in().await;
    let resources = server(&url).await;
    let athlete = session_athlete(&resources, "chat-walks-later-page").await;

    let args = json!({ "provider": SESSION, "limit": 10, "offset": 10, "mode": "summary" });
    turn_with(&resources, athlete, args).await;
    assert_eq!(
        stand_in.session_lists.load(Ordering::SeqCst),
        1,
        "bug: the later page was not read live from the session"
    );
    time::sleep(SETTLE).await;
    assert_eq!(
        walks_recorded(&resources, athlete, SESSION).await,
        [None, None],
        "bug: a read of a later page asked the walks"
    );
}

/// A session read its grant answered in its place proves nothing of the
/// session, and a read of a grant alone has no session to walk: neither asks
/// a walk, under the session or the grant.
#[tokio::test]
async fn a_read_the_session_did_not_answer_asks_nothing() {
    let (url, stand_in) = spawn_stand_in().await;
    let resources = server(&url).await;
    let paired = session_athlete(&resources, "chat-walks-paired").await;
    hold_strava_grant(&resources, paired.0, paired.1).await;
    let grant_only = grant_athlete(&resources).await;

    stand_in.head_complete.store(false, Ordering::SeqCst);
    let served = turn(&resources, paired, SESSION, 10).await;
    assert!(
        served.iter().any(|id| id == GRANT_RIDE),
        "bug: the grant did not answer the head-less session's read: {served:?}"
    );
    assert_eq!(
        turn(&resources, grant_only, oauth_providers::STRAVA, 10).await,
        [GRANT_RIDE],
        "bug: the grant-only athlete's turn did not serve the grant's ride"
    );
    assert_eq!(
        stand_in.grant_lists.load(Ordering::SeqCst),
        2,
        "bug: the grant was not read for each of the two turns"
    );
    time::sleep(SETTLE).await;
    for (athlete, backend) in [
        (paired, SESSION),
        (paired, oauth_providers::STRAVA),
        (grant_only, oauth_providers::STRAVA),
    ] {
        assert_eq!(
            walks_recorded(&resources, athlete, backend).await,
            [None, None],
            "bug: a read the session did not answer asked walks under {backend}"
        );
    }

    // The session is one the walks count: its walk was not asked because
    // the read did not ask it, not because none could be.
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert_eq!(
        detail_walk::ask_track_walk(&runtime, paired.0, paired.1, SESSION).await,
        TrackWalkAsked::Started,
        "bug: the paired session is no session the walks count"
    );
}

/// A walk the ledger refuses to record fails its ask, and the turn that
/// asked is answered as it would have been: the ask runs off the turn, and
/// the walk it could record is still recorded.
#[tokio::test]
async fn a_failed_ask_leaves_the_turn_answered() {
    let (url, _stand_in) = spawn_stand_in().await;
    let resources = server(&url).await;
    let athlete = session_athlete(&resources, "chat-walks-refused").await;
    refuse_walk_rows_on(&resources, BackfillJobKind::TrackWalk, "INSERT").await;

    assert_eq!(
        turn(&resources, athlete, SESSION, 10).await,
        [SESSION_RIDE],
        "bug: a failed walk ask changed the turn's answer"
    );
    let deadline = Instant::now() + ASKED_DEADLINE;
    while walks_recorded(&resources, athlete, SESSION).await[0].is_none() {
        assert!(
            Instant::now() < deadline,
            "bug: the read's ask never recorded the tag walk"
        );
        time::sleep(Duration::from_millis(50)).await;
    }
    time::sleep(SETTLE).await;
    let [_, track] = walks_recorded(&resources, athlete, SESSION).await;
    accept_walk_rows(&resources).await;
    assert_eq!(
        track, None,
        "bug: the refused track walk was recorded anyway"
    );
}

/// An hour in ms: inside the day a failing walk's end holds re-asks off for.
const INSIDE_THE_BACK_OFF_MS: i64 = 3_600_000;

/// A day and an hour in ms: past that day.
const PAST_THE_BACK_OFF_MS: i64 = 25 * 3_600_000;

/// File for the athlete a history walk of `backend` that ended incomplete at
/// `ended_at_ms` (`None`: its credential died, no end stamped), its cursor
/// six years back, sixty months and seven rows done; the cursor.
async fn incomplete_walk(
    resources: &Arc<ServerContext>,
    (user_id, tenant): (Uuid, TenantId),
    backend: &str,
    ended_at_ms: Option<i64>,
) -> i64 {
    let row = ActivityBackfillJobRow {
        provider: backend.to_owned(),
        ..job_row(user_id, tenant, 0)
    };
    assert!(
        resources
            .common
            .repos
            .activity_backfill_jobs
            .record_backfill_job(&row)
            .await
            .unwrap(),
        "bug: the ledger records the walk"
    );
    let cursor = (Utc::now() - ChronoDuration::days(6 * 365)).timestamp();
    update_walk_row(
        resources,
        user_id,
        "UPDATE activity_backfill_jobs SET outcome = 'incomplete', before_ts = $2, \
         months_done = 60, rows_landed = 7, ended_at_ms = $3, leased_until_ms = 0, \
         lease_token = NULL WHERE user_id = $1",
        &[cursor, ended_at_ms.unwrap_or_default()],
    )
    .await;
    if ended_at_ms.is_none() {
        update_walk_row(
            resources,
            user_id,
            "UPDATE activity_backfill_jobs SET ended_at_ms = NULL WHERE user_id = $1",
            &[],
        )
        .await;
    }
    cursor
}

/// The athlete's history walk of `backend` as it stands.
async fn history_walk(
    resources: &Arc<ServerContext>,
    (user_id, tenant): (Uuid, TenantId),
    backend: &str,
) -> DetailWalkRow {
    resources
        .common
        .repos
        .activity_backfill_jobs
        .detail_walk_row(user_id, tenant, backend, BackfillJobKind::HistoryWalk)
        .await
        .unwrap()
        .expect("bug: the history walk's row is kept")
}

/// Wait until the athlete's history walk of the session is owed again; it.
async fn walk_owed_again(
    resources: &Arc<ServerContext>,
    athlete: (Uuid, TenantId),
) -> DetailWalkRow {
    let deadline = Instant::now() + ASKED_DEADLINE;
    loop {
        let row = history_walk(resources, athlete, SESSION).await;
        if row.state == BackfillJobState::Owed {
            return row;
        }
        assert!(
            Instant::now() < deadline,
            "bug: a chat read of the session's list head did not resume its incomplete history walk"
        );
        time::sleep(Duration::from_millis(50)).await;
    }
}

/// A chat read of the session's list head resumes the session's history walk
/// that ended incomplete a day ago, and the one whose credential died (no
/// end stamped): each is owed again from its cursor, with its counters,
/// never from the head. One that ended an hour ago waits out its day: the
/// read asks its walks (the track walk, asked last, is recorded) and leaves
/// it incomplete where it stopped.
#[tokio::test]
async fn a_chat_read_that_carried_the_list_head_resumes_an_incomplete_history_walk() {
    let (url, _stand_in) = spawn_stand_in().await;
    let resources = server(&url).await;
    let past = session_athlete(&resources, "chat-resume-past").await;
    let unstamped = session_athlete(&resources, "chat-resume-unstamped").await;
    let inside = session_athlete(&resources, "chat-resume-inside").await;
    let now_ms = Utc::now().timestamp_millis();
    let past_cursor = incomplete_walk(
        &resources,
        past,
        SESSION,
        Some(now_ms - PAST_THE_BACK_OFF_MS),
    )
    .await;
    let unstamped_cursor = incomplete_walk(&resources, unstamped, SESSION, None).await;
    let inside_cursor = incomplete_walk(
        &resources,
        inside,
        SESSION,
        Some(now_ms - INSIDE_THE_BACK_OFF_MS),
    )
    .await;

    for (athlete, cursor) in [(past, past_cursor), (unstamped, unstamped_cursor)] {
        assert_eq!(
            turn(&resources, athlete, SESSION, 10).await,
            [SESSION_RIDE],
            "bug: the turn did not serve the session's ride"
        );
        let row = walk_owed_again(&resources, athlete).await;
        assert_eq!(
            row.before_ts,
            Some(cursor),
            "bug: the resumed walk starts over from the head"
        );
        assert_eq!(
            (row.progress.units_done, row.progress.rows_landed),
            (60, 7),
            "bug: the resumed walk dropped its counters"
        );
    }

    assert_eq!(
        turn(&resources, inside, SESSION, 10).await,
        [SESSION_RIDE],
        "bug: the turn did not serve the session's ride"
    );
    let deadline = Instant::now() + ASKED_DEADLINE;
    while walks_recorded(&resources, inside, SESSION).await[1].is_none() {
        assert!(
            Instant::now() < deadline,
            "bug: the read never asked the session's track walk"
        );
        time::sleep(Duration::from_millis(50)).await;
    }
    let row = history_walk(&resources, inside, SESSION).await;
    assert_eq!(
        (row.state, row.before_ts),
        (BackfillJobState::Incomplete, Some(inside_cursor)),
        "bug: a walk inside its back-off was owed again, or lost its cursor"
    );
}

/// A grant is no session the walk windows count: a chat read of a
/// grant-only athlete's list head leaves that grant's history walk that
/// ended incomplete a day ago where it stopped. Only a counted session's
/// head refresh resumes a walk; a grant's walk is asked again by a connect,
/// a first sync or a depth miss.
#[tokio::test]
async fn a_chat_read_of_a_grant_alone_leaves_its_incomplete_history_walk() {
    let (url, stand_in) = spawn_stand_in().await;
    let resources = server(&url).await;
    let grant_only = grant_athlete(&resources).await;
    let now_ms = Utc::now().timestamp_millis();
    let cursor = incomplete_walk(
        &resources,
        grant_only,
        oauth_providers::STRAVA,
        Some(now_ms - PAST_THE_BACK_OFF_MS),
    )
    .await;

    assert_eq!(
        turn(&resources, grant_only, oauth_providers::STRAVA, 10).await,
        [GRANT_RIDE],
        "bug: the grant-only athlete's turn did not serve the grant's ride"
    );
    assert_eq!(
        stand_in.grant_lists.load(Ordering::SeqCst),
        1,
        "bug: the turn did not read the grant's list live"
    );
    time::sleep(SETTLE).await;
    let row = history_walk(&resources, grant_only, oauth_providers::STRAVA).await;
    assert_eq!(
        (row.state, row.before_ts),
        (BackfillJobState::Incomplete, Some(cursor)),
        "bug: a grant's head read resumed its incomplete history walk"
    );
}
