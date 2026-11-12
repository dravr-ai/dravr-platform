// ABOUTME: A history walk of a usr/pwd Garmin session beside a Garmin grant: 36 months a step, and the session stays
// ABOUTME: Pins the walk under the session key, its steps and progress, the wellness before the grant, the dead end, the resume

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! History walks of a session beside a Garmin grant (carnet#1482).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// The usr/pwd session is the athlete's primary source and the Garmin grant
// its fallback (carnet#1494); a history walk reads the session first even
// while the grant is alive. Asked when a read finds depth missing, the walk
// pages the session itself back, 36 months a step, stores what it reads
// under the session's key (served from either half of the pair), copies the
// year of wellness before the grant, and ends at the first empty step,
// keeping its row as walked. The session stays (B1). A session that dies
// mid-walk ends it incomplete and is flagged for its sign-in, as every
// background read of the athlete's primary is.
//
// The scraper is a loopback stand-in (a test double, per the repo's mock
// rule) that answers by session and records every call it serves; Garmin's
// OAuth is the dev fixture's stand-in, as in the Garmin API e2e suite.
#![cfg(all(feature = "provider-garmin", feature = "health-sync"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::garmin_sign_in_test::{
    athlete, calls_matching, connections, hold_grant, hold_session, reconnect_notices, server,
    session_header, session_json, session_token_held, sign_in, workout, Calls, LIVE_SESSION,
    SIGNED_IN_SESSION,
};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use chrono::{Duration as ChronoDuration, NaiveDate, Utc};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::{
    Activity, ConnectionStatus, DataSource, DeviceType, StoredRecoveryMetrics, TenantId,
};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::{
    ActivityBackfillJobRow, BackfillJobClaim, BackfillJobKind, BackfillJobState, JobLease,
    JobProgress, WellnessWriteMode,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_notifications::TenantId as CommereTenantId;
use pierre_providers::core::ActivityQueryParams;
use pierre_tool_runtime::activity_backfill::history_walk::{
    start_history_walk, SCIOTTE_WALK_STEP_MONTHS, WHOLE_ACCOUNT_AFTER,
};
use pierre_tool_runtime::activity_backfill_resume::resume_backfill_jobs;
use pierre_tool_runtime::activity_fetch::serve_stale_activities;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::time::sleep;
use uuid::Uuid;

/// The three workouts the stand-in's account holds, oldest first: one in
/// each of the walk's first three 36-month steps, so the fourth step finds
/// nothing and ends the walk.
const OLD_RUN: &str = "7000000001";
const MID_RIDE: &str = "7000000002";
const RECENT_RIDE: &str = "7000000003";

/// How long a detached walk gets to end before the test gives up.
pub(crate) const WALK_DEADLINE: Duration = Duration::from_secs(20);

/// The stand-in scraper's state: what it served, whether a read of the
/// account finds the session expired, and whether the next one fails.
#[derive(Clone)]
struct Scraper {
    calls: Calls,
    switch: ScraperSwitch,
}

/// What a test flips on the stand-in while it runs.
#[derive(Clone, Default)]
pub(crate) struct ScraperSwitch {
    /// Every read of the account answers that the session expired.
    pub(crate) expired: Arc<AtomicBool>,
    /// The next activity read answers a server error, once.
    pub(crate) fail_next: Arc<AtomicBool>,
    /// Every daily-summary read answers a server error.
    pub(crate) fail_summaries: Arc<AtomicBool>,
    /// The daily-summary read of this one day answers a server error.
    pub(crate) fail_summary_on: Arc<Mutex<Option<NaiveDate>>>,
}

/// A stand-in for the scraper service over a Garmin account holding a run
/// eight years old, a ride four years old and a ride three days old, served
/// within each read's `after`/`before` bounds. `expired` makes every read of
/// the account answer that the session expired, as the ~2 h Garmin web
/// session does mid-copy.
async fn spawn_scraper(expired: bool) -> (String, Calls) {
    let switch = ScraperSwitch::default();
    switch.expired.store(expired, Ordering::SeqCst);
    let (url, calls) = spawn_switchable_scraper(switch).await;
    (url, calls)
}

/// [`spawn_scraper`], answering as `switch` says at each read. Every day's
/// summary holds a resting heart rate.
pub(crate) async fn spawn_switchable_scraper(switch: ScraperSwitch) -> (String, Calls) {
    let calls: Calls = Arc::new(Mutex::new(Vec::new()));
    let state = Scraper {
        calls: Arc::clone(&calls),
        switch,
    };
    let now = Utc::now();
    let account = Arc::new(vec![
        (RECENT_RIDE, "ride", now - ChronoDuration::days(3)),
        (MID_RIDE, "ride", now - ChronoDuration::days(4 * 365)),
        (OLD_RUN, "run", now - ChronoDuration::days(8 * 365)),
    ]);
    let app = Router::new()
        .route(
            "/auth/login-with-credentials",
            post(
                |State(scraper): State<Scraper>, Json(body): Json<Value>| async move {
                    scraper
                        .calls
                        .lock()
                        .unwrap()
                        .push(format!("login {}", body["provider"]));
                    Json(json!({
                        "status": "authenticated",
                        "session_id": SIGNED_IN_SESSION,
                        "provider": body["provider"],
                    }))
                },
            ),
        )
        .route(
            "/auth/sessions/{session_id}/export",
            get(|Path(session_id): Path<String>| async move {
                Json(json!({ "session": session_json(&session_id) }))
            }),
        )
        .route(
            "/auth/sessions/{session_id}",
            delete(
                |State(scraper): State<Scraper>, Path(session_id): Path<String>| async move {
                    scraper
                        .calls
                        .lock()
                        .unwrap()
                        .push(format!("drop {session_id}"));
                    StatusCode::NO_CONTENT
                },
            ),
        )
        .route(
            "/auth/import-session",
            post(|Json(body): Json<Value>| async move {
                Json(json!({ "session_id": body["session"]["session_id"] }))
            }),
        )
        .route(
            "/api/athlete",
            get(|| async { Json(json!({ "id": "garmin-athlete", "display_name": "Gar Min" })) }),
        )
        .route(
            "/api/activities",
            get(
                move |State(scraper): State<Scraper>,
                      headers: HeaderMap,
                      RawQuery(query): RawQuery| {
                    let account = Arc::clone(&account);
                    async move {
                        let session = session_header(&headers);
                        let query = query.unwrap_or_default();
                        scraper
                            .calls
                            .lock()
                            .unwrap()
                            .push(format!("activities {session} {query}"));
                        if scraper.switch.fail_next.swap(false, Ordering::SeqCst) {
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(json!({ "error": "internal", "message": "scraper hiccup" })),
                            );
                        }
                        if scraper.switch.expired.load(Ordering::SeqCst) {
                            return (
                                StatusCode::UNAUTHORIZED,
                                Json(json!({
                                    "error": "session_expired",
                                    "message": "the Garmin web session expired"
                                })),
                            );
                        }
                        let after = query_bound(&query, "after");
                        let before = query_bound(&query, "before");
                        let activities: Vec<Value> = account
                            .iter()
                            .filter(|(_, _, start)| {
                                after.is_none_or(|after| start.timestamp() >= after)
                                    && before.is_none_or(|before| start.timestamp() < before)
                            })
                            .map(|(id, sport, start)| workout(id, sport, &start.to_rfc3339()))
                            .collect();
                        (
                            StatusCode::OK,
                            Json(json!({
                                "count": activities.len(),
                                "activities": activities,
                                "head_complete": true
                            })),
                        )
                    }
                },
            ),
        )
        .route(
            "/api/daily-summary",
            get(
                |State(scraper): State<Scraper>, headers: HeaderMap, RawQuery(query): RawQuery| async move {
                    let session = session_header(&headers);
                    let query = query.unwrap_or_default();
                    let date = query
                        .split('&')
                        .find_map(|pair| pair.strip_prefix("date="))
                        .unwrap_or_default()
                        .to_owned();
                    scraper
                        .calls
                        .lock()
                        .unwrap()
                        .push(format!("summary {session} {date}"));
                    if scraper.switch.expired.load(Ordering::SeqCst) {
                        return (
                            StatusCode::UNAUTHORIZED,
                            Json(json!({
                                "error": "session_expired",
                                "message": "the Garmin web session expired"
                            })),
                        );
                    }
                    let failing_day = *scraper.switch.fail_summary_on.lock().unwrap();
                    if scraper.switch.fail_summaries.load(Ordering::SeqCst)
                        || failing_day.is_some_and(|day| day.to_string() == date)
                    {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({ "error": "internal", "message": "scraper hiccup" })),
                        );
                    }
                    (
                        StatusCode::OK,
                        Json(json!({ "date": date, "provider": "garmin", "resting_heart_rate": 48 })),
                    )
                },
            ),
        )
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), calls)
}

/// The epoch-seconds bound `name` an activities read's query names.
fn query_bound(query: &str, name: &str) -> Option<i64> {
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix(&format!("{name}=")))
        .and_then(|value| value.parse().ok())
}

/// What the ledger holds for the session's history walk.
pub(crate) async fn job_state(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> Option<BackfillJobState> {
    job_state_of(resources, user_id, tenant, BackfillJobKind::HistoryWalk).await
}

/// What the ledger holds for the session's `kind` job.
pub(crate) async fn job_state_of(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    kind: BackfillJobKind,
) -> Option<BackfillJobState> {
    resources
        .common
        .repos
        .activity_backfill_jobs
        .backfill_job_state(user_id, tenant, oauth_providers::SCIOTTE_GARMIN, kind)
        .await
        .unwrap()
}

/// Make every job row of the athlete old enough for the resume sweep, which
/// leaves a row alone for two minutes after it was asked so it never races
/// the ask's own spawn.
pub(crate) async fn age_job_rows(resources: &Arc<ServerContext>, user_id: Uuid) {
    let sql = "UPDATE activity_backfill_jobs SET created_at_ms = 0 WHERE user_id = $1";
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query(sql)
                .bind(user_id.to_string())
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            sqlx::query(sql)
                .bind(user_id.to_string())
                .execute(pg.pool())
                .await
                .unwrap();
        }
    }
}

/// Run the athlete's walk the way the resume sweep does, one step a pass,
/// until its row ends — walked, or incomplete — and return how it ended.
pub(crate) async fn walk_ended(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> BackfillJobState {
    let deadline = Instant::now() + WALK_DEADLINE;
    loop {
        let state = job_state(resources, user_id, tenant).await;
        if let Some(ended @ (BackfillJobState::Walked | BackfillJobState::Incomplete)) = state {
            return ended;
        }
        assert!(
            Instant::now() < deadline,
            "the walk did not end within {WALK_DEADLINE:?} (row: {state:?})"
        );
        age_job_rows(resources, user_id).await;
        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        resume_backfill_jobs(runtime).await.unwrap();
        sleep(Duration::from_millis(50)).await;
    }
}

/// The walk's progress on its row.
pub(crate) async fn walk_progress(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> JobProgress {
    resources
        .common
        .repos
        .activity_backfill_jobs
        .job_progress(
            user_id,
            tenant,
            oauth_providers::SCIOTTE_GARMIN,
            BackfillJobKind::HistoryWalk,
        )
        .await
        .unwrap()
        .expect("the walk's row is kept")
}

/// Ask for the athlete's Garmin history walk the way a read that finds
/// depth missing does; whether a walk was started.
pub(crate) async fn ask_walk(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> bool {
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    start_history_walk(&runtime, user_id, tenant, oauth_providers::GARMIN).await
}

/// The athlete's cached activities under `key`.
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

/// The type of every notification the athlete was sent.
pub(crate) async fn notification_types(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> Vec<String> {
    let service = resources
        .common
        .notification_service
        .as_ref()
        .expect("the test server wires notifications");
    let (rows, _, _) = service
        .list_notifications(
            user_id,
            CommereTenantId(tenant.as_uuid()),
            50,
            0,
            None,
            false,
        )
        .await
        .unwrap();
    rows.into_iter().map(|row| row.notification_type).collect()
}

/// The activity ids the Garmin grant's stored window serves the athlete,
/// from 2010 on: its pushes and what the session beside it walked
/// (`backend_resolver::history_keys`).
async fn served_ids(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> Vec<String> {
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let params = ActivityQueryParams {
        limit: Some(50),
        offset: None,
        before: None,
        after: Some(1_262_304_000),
    };
    let mut ids: Vec<String> =
        serve_stale_activities(&runtime, oauth_providers::GARMIN, user_id, tenant, &params)
            .await
            .expect("the grant's stored window answers")
            .iter()
            .map(|activity| Activity::id(activity).to_owned())
            .collect();
    ids.sort();
    ids
}

/// The dev athlete's path: a Garmin grant holder whose usr/pwd session
/// stands beside the grant. Their walk reads the session itself, 36 months
/// a step, newest first — every step of it the session's, never the
/// grant's — its rows land under `sciotte_garmin` and are served through the
/// grant's stored window too, and the first empty step ends the walk, whose
/// row is kept as walked with its progress. The session stays for the next
/// walk (B1), and nothing was told.
#[tokio::test]
async fn a_walk_beside_a_grant_reads_the_whole_history_from_the_session_and_keeps_it() {
    let (scraper, calls) = spawn_scraper(false).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    hold_session(&resources, user_id, tenant, SIGNED_IN_SESSION).await;

    assert!(
        ask_walk(&resources, user_id, tenant).await,
        "bug: a depth ask walks the session"
    );
    assert_eq!(
        walk_ended(&resources, user_id, tenant).await,
        BackfillJobState::Walked,
        "bug: the walk reaches the end of the history"
    );

    assert_eq!(
        rows_under(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        3,
        "bug: the walk stores the whole account under the session's key"
    );
    let reads = calls_matching(&calls, &format!("activities {SIGNED_IN_SESSION}"));
    assert_eq!(
        reads.len(),
        4,
        "bug: three steps land a workout each and the fourth, empty, ends the walk: {reads:?}"
    );
    assert!(
        !reads[0].contains("before="),
        "bug: the first step reads the head of the list: {reads:?}"
    );
    assert!(
        reads[1..].iter().all(|read| read.contains("before=")),
        "bug: every later step reads below the last one: {reads:?}"
    );
    assert!(
        reads
            .iter()
            .all(|read| !read.contains(&format!("after={WHOLE_ACCOUNT_AFTER}"))),
        "bug: a step reads 36 months, never the whole account at once: {reads:?}"
    );
    let progress = walk_progress(&resources, user_id, tenant).await;
    assert_eq!(
        progress.months_done,
        4 * SCIOTTE_WALK_STEP_MONTHS,
        "bug: each step adds its 36 months to the walk's progress"
    );
    assert_eq!(
        progress.rows_landed, 3,
        "bug: the progress counts every row the steps landed"
    );
    assert!(
        progress.secs_per_unit.is_some(),
        "bug: a step measures the seconds a month took"
    );
    assert_eq!(
        progress.months_total, None,
        "bug: a session names no floor of its own, so the walk's length is unknown"
    );
    let coverage = resources
        .common
        .repos
        .activity_cache
        .get_backfill_coverage(user_id, &tenant, oauth_providers::SCIOTTE_GARMIN)
        .await
        .unwrap()
        .expect("the walk records its coverage");
    assert!(
        coverage.hit_feed_end,
        "bug: a walk that reached the end of the history says so"
    );
    assert!(
        coverage.oldest_reached_ts < (Utc::now() - ChronoDuration::days(8 * 365)).timestamp(),
        "bug: the walk reached below the account's oldest workout"
    );
    assert_eq!(
        served_ids(&resources, user_id, tenant).await,
        vec![OLD_RUN, MID_RIDE, RECENT_RIDE],
        "bug: the walked history is served through the grant's window"
    );
    // The wellness half read the session over the year before the grant's
    // first day, each day once, and never the grant's day or after: the
    // push holds that day on.
    let today = Utc::now().date_naive();
    let summary_days: Vec<NaiveDate> =
        calls_matching(&calls, &format!("summary {SIGNED_IN_SESSION} "))
            .iter()
            .map(|call| {
                let date = call.rsplit(' ').next().unwrap_or_default();
                NaiveDate::parse_from_str(date, "%Y-%m-%d")
                    .unwrap_or_else(|e| panic!("summary read of a day: {call}: {e}"))
            })
            .collect();
    let long_before = today - ChronoDuration::days(200);
    assert!(
        summary_days.contains(&long_before),
        "bug: the walk reads wellness far older than the sync's two weeks"
    );
    assert!(
        summary_days.iter().all(|day| *day < today),
        "bug: the walk never reads a day the push holds"
    );
    assert_eq!(
        summary_days.len(),
        365,
        "bug: a year of days before the grant, each read once"
    );
    assert_eq!(
        summary_days.iter().min().copied(),
        Some(today - ChronoDuration::days(365)),
        "bug: the read starts a year before the grant's first day"
    );
    let mornings = resources
        .common
        .repos
        .recovery
        .get_recovery_metrics(
            user_id,
            &tenant,
            Utc::now() - ChronoDuration::days(400),
            Utc::now() + ChronoDuration::days(1),
        )
        .await
        .unwrap();
    assert!(
        !mornings.is_empty(),
        "bug: the mornings before the grant are copied"
    );
    assert!(
        mornings.iter().any(|morning| morning.date == long_before),
        "bug: a morning 200 days before the grant is copied: {:?}",
        mornings.iter().map(|m| m.date).min()
    );
    assert!(
        mornings.iter().all(|morning| morning.date < today),
        "bug: the walk never writes a day the push holds: {:?}",
        mornings.iter().map(|m| m.date).collect::<Vec<_>>()
    );

    assert!(
        session_token_held(&resources, user_id, tenant).await,
        "bug: the walked session's token row stays for the next walk (B1)"
    );
    let mut held = connections(&resources, user_id, tenant).await;
    held.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        held,
        vec![
            (oauth_providers::GARMIN.to_owned(), ConnectionStatus::Active),
            (
                oauth_providers::SCIOTTE_GARMIN.to_owned(),
                ConnectionStatus::Active
            ),
        ],
        "bug: the session's connection stays beside the grant (B1)"
    );
    assert_eq!(
        calls_matching(&calls, &format!("drop {SIGNED_IN_SESSION}")),
        Vec::<String>::new(),
        "bug: the scraper is never told to drop a walked session"
    );
    assert_eq!(
        notification_types(&resources, user_id, tenant).await,
        Vec::<String>::new(),
        "the walk is told to nobody"
    );
}

/// The resting heart rate every morning the push stores carries, apart from
/// the one the usr/pwd session's summaries answer.
const PUSHED_RESTING_HR: u32 = 41;

/// Store, under the Garmin API's own data source as the push ingest files
/// them, one morning on each of `days`.
async fn push_mornings(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    days: &[NaiveDate],
) -> String {
    let repos = &resources.common.repos;
    let source = repos
        .data_sources
        .upsert_data_source(
            &tenant,
            &DataSource {
                id: String::new(),
                user_id: user_id.to_string(),
                provider: oauth_providers::GARMIN.to_owned(),
                device_model: None,
                software_version: None,
                // The push ingest's marker on its data source.
                source: Some(oauth_providers::GARMIN.to_owned()),
                device_type: DeviceType::Watch,
                original_source_name: Some("Garmin Connect".to_owned()),
            },
        )
        .await
        .unwrap();
    for day in days {
        repos
            .recovery
            .upsert_recovery_metrics(
                &tenant,
                &StoredRecoveryMetrics {
                    id: format!("pushed-{day}"),
                    user_id: user_id.to_string(),
                    data_source_id: source.clone(),
                    date: *day,
                    recovery_score: None,
                    readiness_score: None,
                    hrv_ms: None,
                    hrv_rmssd: None,
                    resting_heart_rate: Some(PUSHED_RESTING_HR),
                    stress_score: None,
                    body_battery: None,
                    spo2: None,
                    respiratory_rate: None,
                    skin_temp_deviation: None,
                    daily_strain: None,
                    athlete_note: None,
                    source_name: oauth_providers::GARMIN.to_owned(),
                    recorded_at: Utc::now(),
                },
                WellnessWriteMode::FullDay,
                Utc::now(),
            )
            .await
            .unwrap();
    }
    source
}

/// The grant was reconnected after its pushes stored days: the connection's
/// `connected_at` is today, later than what the push holds. The walk's
/// wellness half still stops before the first day the push stored, and every
/// pushed morning is left as the push wrote it.
#[tokio::test]
async fn a_reconnected_grants_pushed_days_are_never_overwritten_by_the_walk() {
    let (scraper, calls) = spawn_scraper(false).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    let today = Utc::now().date_naive();
    let pushed_days: Vec<NaiveDate> = (3..=10)
        .map(|back| today - ChronoDuration::days(back))
        .collect();
    let first_pushed = today - ChronoDuration::days(10);
    let source = push_mornings(&resources, user_id, tenant, &pushed_days).await;
    // The reconnect: the grant's connection is (re)registered today.
    hold_grant(&resources, user_id, tenant).await;
    hold_session(&resources, user_id, tenant, SIGNED_IN_SESSION).await;

    assert!(
        ask_walk(&resources, user_id, tenant).await,
        "bug: the session is walked"
    );
    assert_eq!(
        walk_ended(&resources, user_id, tenant).await,
        BackfillJobState::Walked,
        "bug: the walk reaches the end of the history"
    );

    let summary_days: Vec<NaiveDate> =
        calls_matching(&calls, &format!("summary {SIGNED_IN_SESSION} "))
            .iter()
            .map(|call| {
                let date = call.rsplit(' ').next().unwrap_or_default();
                NaiveDate::parse_from_str(date, "%Y-%m-%d")
                    .unwrap_or_else(|e| panic!("summary read of a day: {call}: {e}"))
            })
            .collect();
    assert!(
        !summary_days.is_empty(),
        "bug: the walk still reads the session's wellness before the push"
    );
    assert!(
        summary_days.iter().all(|day| *day < first_pushed),
        "bug: the walk reads a day the push already stored, though the reconnect restamped the grant: {:?}",
        summary_days.iter().max()
    );
    let mornings = resources
        .common
        .repos
        .recovery
        .get_recovery_metrics(
            user_id,
            &tenant,
            Utc::now() - ChronoDuration::days(400),
            Utc::now() + ChronoDuration::days(1),
        )
        .await
        .unwrap();
    for day in &pushed_days {
        let morning = mornings
            .iter()
            .find(|morning| morning.date == *day)
            .unwrap_or_else(|| panic!("bug: the pushed morning of {day} is gone"));
        assert_eq!(
            (morning.data_source_id.as_str(), morning.resting_heart_rate),
            (source.as_str(), Some(PUSHED_RESTING_HR)),
            "bug: the walk overwrote the pushed morning of {day}"
        );
    }
    assert!(
        mornings.iter().any(|morning| morning.date < first_pushed),
        "bug: the mornings before the push are still copied"
    );
}

/// The Garmin web session dies mid-walk: the walk ends incomplete, and the
/// session, the athlete's primary source (carnet#1494), is flagged and its
/// reconnect told once, as every background read of it is. The grant stands
/// and the session is kept (B1); once the athlete signs in again, the next
/// walk replaces the incomplete row and lands.
#[tokio::test]
async fn a_session_expiring_mid_walk_ends_incomplete_and_asks_for_its_sign_in() {
    let switch = ScraperSwitch::default();
    switch.expired.store(true, Ordering::SeqCst);
    let (scraper, calls) = spawn_switchable_scraper(switch.clone()).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    hold_session(&resources, user_id, tenant, SIGNED_IN_SESSION).await;

    assert!(
        ask_walk(&resources, user_id, tenant).await,
        "bug: the session is walked"
    );
    assert_eq!(
        walk_ended(&resources, user_id, tenant).await,
        BackfillJobState::Incomplete,
        "bug: the job says the walk did not finish"
    );

    assert!(
        !calls_matching(&calls, &format!("activities {SIGNED_IN_SESSION}")).is_empty(),
        "the walk read the session"
    );
    assert_eq!(
        reconnect_notices(&resources, user_id, tenant).await,
        1,
        "bug: the athlete's dead primary asks for its sign-in once"
    );
    let mut held = connections(&resources, user_id, tenant).await;
    held.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        held,
        vec![
            (oauth_providers::GARMIN.to_owned(), ConnectionStatus::Active),
            (
                oauth_providers::SCIOTTE_GARMIN.to_owned(),
                ConnectionStatus::NeedsReauth
            ),
        ],
        "bug: the grant stands, and the dead session is kept, flagged for its sign-in"
    );
    assert_eq!(
        rows_under(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        0,
        "bug: nothing landed under the session's key"
    );

    // "Continue": the athlete signs in again on a session that lives, and
    // the next walk replaces the incomplete row and lands.
    switch.expired.store(false, Ordering::SeqCst);
    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(status, 200, "{body}");
    assert!(
        ask_walk(&resources, user_id, tenant).await,
        "bug: an ended walk never blocks the next one"
    );
    assert_eq!(
        walk_ended(&resources, user_id, tenant).await,
        BackfillJobState::Walked,
        "bug: the next walk reaches the end of the history"
    );
    assert_eq!(
        rows_under(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        3,
        "bug: the walk lands under the session's key"
    );
}

/// The job row a history walk of the session records.
pub(crate) fn job_row(
    user_id: Uuid,
    tenant: TenantId,
    created_at_ms: i64,
) -> ActivityBackfillJobRow {
    ActivityBackfillJobRow {
        id: Uuid::new_v4().to_string(),
        tenant_id: tenant,
        user_id,
        provider: oauth_providers::SCIOTTE_GARMIN.to_owned(),
        after_ts: Some(WHOLE_ACCOUNT_AFTER),
        before_ts: None,
        fetch_limit: Some(2_000),
        conversation_id: None,
        created_at_ms,
        leased_until_ms: 0,
        attempts: 1,
        lease_token: None,
        kind: BackfillJobKind::HistoryWalk,
        progress: JobProgress::default(),
        turn_id: None,
        asked_message_id: None,
    }
}

/// An instance died holding a walk: the resume sweep re-runs it on the
/// session the row names — never the grant routing would pick — step after
/// step, and the session stays when the walk ends (B1).
#[tokio::test]
async fn a_resumed_walk_reads_the_session_and_keeps_it() {
    let (scraper, calls) = spawn_scraper(false).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;
    let long_ago = Utc::now().timestamp_millis() - 600_000;
    assert!(
        resources
            .common
            .repos
            .activity_backfill_jobs
            .record_backfill_job(&job_row(user_id, tenant, long_ago))
            .await
            .unwrap(),
        "bug: the ledger records the walk"
    );

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let resumed = resume_backfill_jobs(runtime).await.unwrap();
    assert_eq!(resumed, 1, "bug: the sweep resumes the owed walk");
    assert_eq!(
        walk_ended(&resources, user_id, tenant).await,
        BackfillJobState::Walked,
        "bug: the resumed walk reaches the end of the history"
    );

    assert_eq!(
        calls_matching(&calls, &format!("activities {LIVE_SESSION}")).len(),
        4,
        "bug: every step of the resumed walk reads the session it names"
    );
    assert!(
        calls_matching(&calls, "activities")
            .iter()
            .all(|call| call.starts_with(&format!("activities {LIVE_SESSION}"))),
        "bug: no step reads anything but the session"
    );
    assert_eq!(
        rows_under(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        3,
        "bug: the resumed walk lands under the session's key"
    );
    assert!(
        session_token_held(&resources, user_id, tenant).await,
        "bug: the walked session stays (B1)"
    );
}

/// The ledger keeps an ended walk's row out of every sweep: an incomplete
/// row is neither claimed nor reaped, however old.
#[tokio::test]
async fn an_incomplete_walk_row_is_never_claimed_or_reaped() {
    let resources = server(common::test_server_config()).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    let ledger = &resources.common.repos.activity_backfill_jobs;
    let long_ago = Utc::now().timestamp_millis() - 600_000;
    assert!(
        ledger
            .record_backfill_job(&job_row(user_id, tenant, long_ago))
            .await
            .unwrap(),
        "bug: the ledger records the walk"
    );
    let now_ms = Utc::now().timestamp_millis();
    let claim = BackfillJobClaim {
        now_ms,
        queued_older_than_ms: 0,
        lease_ms: 60_000,
        max_attempts: 3,
        limit: 10,
    };
    let claimed = ledger.claim_stale_backfill_jobs(claim).await.unwrap();
    let mine = claimed
        .into_iter()
        .find(|row| row.user_id == user_id)
        .expect("an owed row is claimed");
    let lease = JobLease {
        id: mine.id.clone(),
        token: mine.lease_token.clone().unwrap(),
    };
    assert!(
        ledger.end_backfill_job_incomplete(&lease).await.unwrap(),
        "bug: the lease holder ends its job incomplete"
    );
    assert_eq!(
        job_state(&resources, user_id, tenant).await,
        Some(BackfillJobState::Incomplete),
        "bug: the ended walk's row is kept as incomplete"
    );

    let later = BackfillJobClaim {
        now_ms: now_ms + 3_600_000,
        ..claim
    };
    assert!(
        !ledger
            .claim_stale_backfill_jobs(later)
            .await
            .unwrap()
            .iter()
            .any(|row| row.user_id == user_id),
        "bug: an ended walk is never run again by the sweep"
    );
    ledger
        .reap_exhausted_backfill_jobs(now_ms + 3_600_000, 0)
        .await
        .unwrap();
    assert_eq!(
        job_state(&resources, user_id, tenant).await,
        Some(BackfillJobState::Incomplete),
        "bug: the reaper keeps what the card reads"
    );
    assert!(
        ledger
            .record_backfill_job(&job_row(user_id, tenant, now_ms))
            .await
            .unwrap(),
        "bug: a new walk replaces the ended row"
    );
    assert_eq!(
        job_state(&resources, user_id, tenant).await,
        Some(BackfillJobState::Owed),
        "bug: a new ask replaces the ended row"
    );
    assert!(
        !ledger
            .record_backfill_job(&job_row(user_id, tenant, now_ms))
            .await
            .unwrap(),
        "an owed walk is the same job: a second ask records nothing"
    );
}
