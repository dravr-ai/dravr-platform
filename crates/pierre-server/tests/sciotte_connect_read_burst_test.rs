// ABOUTME: carnet#736 — a COROS connect's own reads never press its fresh session with two scrapes at once
// ABOUTME: The proposal takes the in-flight pre-fetch's activities; the health backfill starts after it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Within a second of a COROS two-factor connect, the platform fired three
//! scrapes of the same fresh session at a scraper service that serves two
//! browsers at a time: the activity pre-fetch, the 30-day health backfill and
//! the onboarding proposal's own activity read. The daily-summary read failed
//! and the proposal's read was shed, so the proposal was built on nothing.
//!
//! The scraper here is a loopback stand-in (a test double, per the repo's mock
//! rule) that serves the login, the session export and import, a slow activity
//! list and the daily summaries, and records when each scrape of the session
//! starts and ends. The connect goes through the real routes, and the
//! proposal is asked for the moment the connect answers, while the pre-fetch
//! is still scraping.
//!
//! One test, because `DRAVR_SCIOTTE_REMOTE_URL` and the pre-fetch registry are
//! process-wide.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
#![cfg(feature = "health-sync")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::env;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, RawQuery, State};
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use common::{create_test_server_resources, create_test_user, generate_test_token};
use dravr_sciotte::client::{ENV_AUDIENCE, ENV_REMOTE_URL};
use helpers::axum_test::AxumTestRequest;
use pierre_core::constants::oauth::providers::provider_terms_version;
use pierre_core::constants::oauth_providers::SCIOTTE_COROS;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_agents::build_agents_router;
use pierre_routes_auth::AuthRoutes;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::time::sleep;

/// The session the COROS login signs in to.
const SESSION: &str = "coros-736";

/// How long the stand-in takes over the activity list: long enough that the
/// proposal, asked for the moment the connect answers, lands mid-scrape.
const ACTIVITY_SCRAPE: Duration = Duration::from_millis(1500);

/// How long one daily-summary scrape takes.
const SUMMARY_SCRAPE: Duration = Duration::from_millis(20);

/// How long the backfill gets to finish before the test gives up.
const BACKFILL_DEADLINE: Duration = Duration::from_secs(60);

/// How long no new daily summary must be read for the backfill to count as
/// finished: many summary scrapes' worth of quiet.
const BACKFILL_QUIET: Duration = Duration::from_secs(2);

/// The runs the account holds, newest first: fewer than the pre-fetch asks
/// for, so the pre-fetch reads the whole history.
const RUN_DAYS_AGO: [i64; 3] = [1, 3, 6];

/// One scrape of the session the stand-in served.
#[derive(Debug, Clone)]
struct Scrape {
    path: &'static str,
    started: Instant,
    ended: Option<Instant>,
}

/// What the stand-in saw of the session: every scrape, and the most that ran
/// at once.
#[derive(Debug, Default)]
struct Seen {
    scrapes: Vec<Scrape>,
    running: usize,
    most_at_once: usize,
}

type Shared = Arc<Mutex<Seen>>;

/// Count a scrape of `path` in; returns its index to count it out with.
fn scrape_started(seen: &Shared, headers: &HeaderMap, path: &'static str) -> Option<usize> {
    let session = headers
        .get("x-session-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if session != SESSION {
        return None;
    }
    let mut seen = seen.lock().unwrap();
    seen.running += 1;
    seen.most_at_once = seen.most_at_once.max(seen.running);
    seen.scrapes.push(Scrape {
        path,
        started: Instant::now(),
        ended: None,
    });
    Some(seen.scrapes.len() - 1)
}

fn scrape_ended(seen: &Shared, index: Option<usize>) {
    if let Some(index) = index {
        let mut seen = seen.lock().unwrap();
        seen.running -= 1;
        seen.scrapes[index].ended = Some(Instant::now());
    }
}

fn session_json() -> Value {
    json!({
        "session_id": SESSION,
        "cookies": [],
        "created_at": "2026-10-02T17:07:08Z",
        "expires_at": null
    })
}

fn runs() -> Value {
    let activities: Vec<Value> = RUN_DAYS_AGO
        .iter()
        .map(|days| {
            json!({
                "id": format!("coros-run-{days}"),
                "name": format!("Run {days} days ago"),
                "sport_type": "run",
                "start_date": (Utc::now() - chrono::Duration::days(*days)).to_rfc3339(),
                "duration_seconds": 3000,
                "provider": "coros",
                "distance_meters": 10000.0
            })
        })
        .collect();
    json!({ "count": activities.len(), "activities": activities, "head_complete": true })
}

/// The scraper service stand-in: a COROS login that asks for a code, the
/// session it then exports, and the session's slow activity list and daily
/// summaries.
async fn spawn_scraper(seen: Shared) -> String {
    let app = Router::new()
        .route(
            "/auth/login-with-credentials",
            post(|| async {
                Json(json!({
                    "status": "otp_required",
                    "reason": "COROS sent a verification code",
                    "flow_id": "flow-736",
                    "provider": "coros"
                }))
            }),
        )
        .route(
            "/auth/submit-otp",
            post(|| async {
                Json(json!({
                    "status": "authenticated",
                    "session_id": SESSION,
                    "provider": "coros"
                }))
            }),
        )
        .route(
            "/auth/sessions/{session_id}/export",
            get(|Path(_session_id): Path<String>| async {
                Json(json!({ "session": session_json() }))
            }),
        )
        .route(
            "/auth/import-session",
            post(|| async { Json(json!({ "session_id": SESSION })) }),
        )
        .route(
            "/api/activities",
            get(|State(seen): State<Shared>, headers: HeaderMap| async move {
                let scrape = scrape_started(&seen, &headers, "/api/activities");
                sleep(ACTIVITY_SCRAPE).await;
                scrape_ended(&seen, scrape);
                Json(runs())
            }),
        )
        .route(
            "/api/daily-summary",
            get(
                |State(seen): State<Shared>, headers: HeaderMap, RawQuery(query): RawQuery| async move {
                    let scrape = scrape_started(&seen, &headers, "/api/daily-summary");
                    sleep(SUMMARY_SCRAPE).await;
                    scrape_ended(&seen, scrape);
                    let query = query.unwrap_or_default();
                    let date = query
                        .split('&')
                        .find_map(|pair| pair.strip_prefix("date="))
                        .unwrap_or_default()
                        .to_owned();
                    Json(json!({ "date": date, "provider": "coros", "resting_heart_rate": 48 }))
                },
            ),
        )
        .with_state(seen);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

async fn post_auth(resources: &Arc<ServerContext>, token: &str, path: &str, body: Value) -> Value {
    let resp = AxumTestRequest::post(path)
        .header("authorization", &format!("Bearer {token}"))
        .json(&body)
        .send(AuthRoutes::routes(resources.auth_routes_context()))
        .await;
    assert_eq!(resp.status(), 200, "{path}: {}", resp.text());
    serde_json::from_str(&resp.text()).unwrap()
}

fn summaries_read(seen: &Shared) -> usize {
    seen.lock()
        .unwrap()
        .scrapes
        .iter()
        .filter(|s| s.path == "/api/daily-summary" && s.ended.is_some())
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_coros_connect_reads_its_session_one_scrape_at_a_time() {
    let seen = Shared::default();
    env::set_var(ENV_REMOTE_URL, spawn_scraper(Arc::clone(&seen)).await);
    env::remove_var(ENV_AUDIENCE);

    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();
    if let Some(version) = provider_terms_version(SCIOTTE_COROS) {
        resources
            .common
            .repos
            .users
            .record_provider_terms(user_id, SCIOTTE_COROS, version)
            .await
            .unwrap();
    }
    let token = generate_test_token(&resources, &user).await;

    let login = post_auth(
        &resources,
        &token,
        "/api/providers/sciotte/login",
        json!({
            "email": "alice@acme.com",
            "password": "not-a-real-password",
            "target": "coros",
            "tos_consent": true,
        }),
    )
    .await;
    assert_eq!(login["status"], "otp_required", "{login}");
    let connected = post_auth(
        &resources,
        &token,
        "/api/providers/sciotte/submit-otp",
        json!({ "code": "123456" }),
    )
    .await;
    assert_eq!(connected["status"], "connected", "{connected}");

    // The onboarding screen asks for its proposal the moment the connect
    // answers, while the pre-fetch is still scraping.
    let proposal = AxumTestRequest::get("/api/agents/proposal")
        .header("authorization", &format!("Bearer {token}"))
        .send(build_agents_router::<ServerContext>().with_state(Arc::clone(&resources)))
        .await;
    assert_eq!(proposal.status(), 200, "{}", proposal.text());
    let proposal: Value = serde_json::from_str(&proposal.text()).unwrap();

    // The backfill has finished once it read some days and then went quiet.
    let started = Instant::now();
    let mut read = summaries_read(&seen);
    let mut quiet_since = Instant::now();
    while read == 0 || quiet_since.elapsed() < BACKFILL_QUIET {
        assert!(
            started.elapsed() < BACKFILL_DEADLINE,
            "the backfill never finished: {:?}",
            seen.lock().unwrap()
        );
        sleep(Duration::from_millis(100)).await;
        let now_read = summaries_read(&seen);
        if now_read != read {
            read = now_read;
            quiet_since = Instant::now();
        }
    }
    env::remove_var(ENV_REMOTE_URL);

    // The proposal was built on the pre-fetched runs, not on an empty read.
    assert_eq!(proposal["profile"]["has_profile"], true, "{proposal}");
    assert_eq!(
        proposal["profile"]["total_activities"],
        RUN_DAYS_AGO.len(),
        "{proposal}"
    );
    assert_eq!(proposal["profile"]["primary_sport"], "run", "{proposal}");

    let seen = seen.lock().unwrap();
    let activity_scrapes: Vec<&Scrape> = seen
        .scrapes
        .iter()
        .filter(|s| s.path == "/api/activities")
        .collect();
    assert_eq!(
        activity_scrapes.len(),
        1,
        "the proposal must take the pre-fetch's read, not scrape again: {seen:?}"
    );
    assert_eq!(
        seen.most_at_once, 1,
        "the connect's reads pressed the session with several scrapes at once: {seen:?}"
    );
    let prefetch_ended = activity_scrapes[0].ended.expect("the pre-fetch finished");
    let first_summary = seen
        .scrapes
        .iter()
        .filter(|s| s.path == "/api/daily-summary")
        .map(|s| s.started)
        .min()
        .expect("the backfill read a summary");
    assert!(
        first_summary >= prefetch_ended,
        "the backfill's first daily summary started before the pre-fetch finished: {seen:?}"
    );
}
