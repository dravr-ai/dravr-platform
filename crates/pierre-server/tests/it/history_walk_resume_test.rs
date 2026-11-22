// ABOUTME: A history walk whose scrape fell short of a step's window: what lands, where it reads next, how it ends
// ABOUTME: Pins the bounded step that keeps its rows, the first step that refuses, the resumed cursor and the back-off

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! History walks that fall short of a step's window (carnet#1698).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// A scrape that read some months of a step and then failed on one returns
// what it read, short of its window (`head_complete` false). A step below
// the head lands those rows and reads the next step below the oldest of them;
// the first step, which reads the head, still refuses such a capture. A walk
// that still ended incomplete resumes from its cursor with its counters when
// asked again, and no sooner than a day after its failing end, so a walk that
// keeps failing never storms the scraper.
//
// The scraper is the history walk suite's loopback stand-in (a test double,
// per the repo's mock rule), told where its month walk fails.
#![cfg(all(feature = "provider-garmin", feature = "health-sync"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::garmin_sign_in_test::{
    athlete, calls_matching, hold_grant, hold_session, server, Calls, SIGNED_IN_SESSION,
};
use crate::history_walk_test::{
    age_job_rows, ask_walk, job_row, job_state, paused_retries_due, query_bound, rows_under,
    spawn_switchable_scraper, ScraperSwitch, WALK_DEADLINE,
};

use std::sync::Arc;
use std::time::{Duration, Instant};

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::models::TenantId;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::{BackfillJobKind, BackfillJobState, DetailWalkRow};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::activity_backfill::MAX_BACKFILL_ATTEMPTS;
use pierre_tool_runtime::activity_backfill_resume::resume_backfill_jobs;
use pierre_tool_runtime::runtime::ToolRuntime;
use tokio::time;
use uuid::Uuid;

/// A day and an hour in ms: past the day a failing walk's end holds re-asks
/// off for.
const PAST_THE_BACK_OFF_MS: i64 = 25 * 3_600_000;

/// Run the athlete's walk the way the resume sweep does, one step a pass,
/// every failed step's retry made due at once, until its row ends — walked,
/// or incomplete — and return how it ended.
async fn walk_settled(
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
        paused_retries_due(resources, user_id).await;
        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        resume_backfill_jobs(runtime).await.unwrap();
        time::sleep(Duration::from_millis(50)).await;
    }
}

/// The session's history walk row as it stands.
async fn walk_row(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
) -> DetailWalkRow {
    resources
        .common
        .repos
        .activity_backfill_jobs
        .detail_walk_row(
            user_id,
            tenant,
            oauth_providers::SCIOTTE_GARMIN,
            BackfillJobKind::HistoryWalk,
        )
        .await
        .unwrap()
        .expect("the walk's row is kept")
}

/// Run `sql` over the athlete's history walk row: `$1` its user, then
/// `values` in order.
pub(crate) async fn update_walk_row(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    sql: &str,
    values: &[i64],
) {
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            let mut query = sqlx::query(sql).bind(user_id.to_string());
            for value in values {
                query = query.bind(*value);
            }
            query.execute(sqlite.pool()).await.unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            let mut query = sqlx::query(sql).bind(user_id.to_string());
            for value in values {
                query = query.bind(*value);
            }
            query.execute(pg.pool()).await.unwrap();
        }
    }
}

/// The session reads the stand-in served.
fn session_reads(calls: &Calls) -> Vec<String> {
    calls_matching(calls, &format!("activities {SIGNED_IN_SESSION}"))
}

/// The stored start of the athlete's oldest activity under the session's key.
async fn oldest_stored(resources: &Arc<ServerContext>, user_id: Uuid, tenant: TenantId) -> i64 {
    resources
        .common
        .repos
        .activity_cache
        .oldest_activity_start(user_id, &tenant, oauth_providers::SCIOTTE_GARMIN)
        .await
        .unwrap()
        .expect("a row is stored")
        .epoch_seconds()
}

/// The session's month walk fails a day below the run eight years old: the
/// third step reads that run and falls short of its window. Its run lands,
/// and the next step reads below it, never below the window. That step finds
/// nothing above the failing month, at the oldest stored row: it is tried
/// again on every attempt, and once they are spent the session holds nothing
/// more to give, so the walk ends there, walked, with every row in.
#[tokio::test]
async fn a_bounded_step_short_of_its_window_lands_and_reads_below_its_oldest_row() {
    let switch = ScraperSwitch::default();
    let cut = (Timestamp::now() - Elapsed::days(8 * 365 + 1)).epoch_seconds();
    *switch.short_below.lock().unwrap() = Some(cut);
    let (scraper, calls) = spawn_switchable_scraper(switch).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    hold_session(&resources, user_id, tenant, SIGNED_IN_SESSION).await;

    assert!(
        ask_walk(&resources, user_id, tenant).await,
        "bug: the session is walked"
    );
    assert_eq!(
        walk_settled(&resources, user_id, tenant).await,
        BackfillJobState::Walked,
        "bug: a step short of its window below the head fails the walk"
    );
    assert_eq!(
        rows_under(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        3,
        "bug: the short step's run is not in the store"
    );
    let oldest = oldest_stored(&resources, user_id, tenant).await;
    let below_oldest: usize = session_reads(&calls)
        .iter()
        .filter(|read| query_bound(read, "before") == Some(oldest))
        .count();
    assert_eq!(
        below_oldest,
        usize::try_from(MAX_BACKFILL_ATTEMPTS).unwrap(),
        "bug: the step below the short one reads below its oldest row, once an attempt: {:?}",
        session_reads(&calls)
    );
    let row = walk_row(&resources, user_id, tenant).await;
    assert_eq!(
        row.progress.rows_landed, 3,
        "bug: the short step's row is not counted"
    );
}

/// The first step reads the head: a capture short of it is refused, as
/// every write-through refuses one, so nothing lands and the walk ends
/// incomplete once its attempts are spent. Asked again at once, the walk is
/// held off: its failing end is a day from over. Past that day, the ask
/// resumes it.
#[tokio::test]
async fn the_first_step_refuses_a_short_capture_and_a_re_ask_waits_out_the_day() {
    let switch = ScraperSwitch::default();
    let cut = (Timestamp::now() - Elapsed::days(10)).epoch_seconds();
    *switch.short_below.lock().unwrap() = Some(cut);
    let (scraper, calls) = spawn_switchable_scraper(switch).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    hold_session(&resources, user_id, tenant, SIGNED_IN_SESSION).await;

    assert!(
        ask_walk(&resources, user_id, tenant).await,
        "bug: the session is walked"
    );
    assert_eq!(
        walk_settled(&resources, user_id, tenant).await,
        BackfillJobState::Incomplete,
        "bug: a head the first step never read ends the walk incomplete"
    );
    assert_eq!(
        rows_under(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        0,
        "bug: the first step landed a capture short of its head"
    );
    let reads = session_reads(&calls).len();

    assert!(
        !ask_walk(&resources, user_id, tenant).await,
        "bug: a walk that just failed is asked again at once"
    );
    assert_eq!(
        job_state(&resources, user_id, tenant).await,
        Some(BackfillJobState::Incomplete),
        "bug: the held-off ask replaced the incomplete row"
    );
    assert_eq!(
        session_reads(&calls).len(),
        reads,
        "bug: the held-off ask read the scraper"
    );

    update_walk_row(
        &resources,
        user_id,
        "UPDATE activity_backfill_jobs SET ended_at_ms = $2 WHERE user_id = $1",
        &[Timestamp::now().epoch_millis() - PAST_THE_BACK_OFF_MS],
    )
    .await;
    assert!(
        ask_walk(&resources, user_id, tenant).await,
        "bug: a day past its failing end, the walk is asked again"
    );
    assert_eq!(
        job_state(&resources, user_id, tenant).await,
        Some(BackfillJobState::Owed),
        "bug: the resumed walk is owed"
    );
}

/// The first step reads nothing above a month that failed: an empty capture
/// short of its head is no end of the history, and the walk never reads as
/// walked on it.
#[tokio::test]
async fn an_empty_first_step_short_of_its_head_never_ends_the_walk() {
    let switch = ScraperSwitch::default();
    let cut = (Timestamp::now() - Elapsed::hours(1)).epoch_seconds();
    *switch.short_below.lock().unwrap() = Some(cut);
    let (scraper, _calls) = spawn_switchable_scraper(switch).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    hold_session(&resources, user_id, tenant, SIGNED_IN_SESSION).await;

    assert!(
        ask_walk(&resources, user_id, tenant).await,
        "bug: the session is walked"
    );
    assert_eq!(
        walk_settled(&resources, user_id, tenant).await,
        BackfillJobState::Incomplete,
        "bug: an empty capture short of its head ended the walk as walked"
    );
}

/// A walk that ended incomplete a day ago, its cursor six years back, sixty
/// months and seven rows done: asked again, it resumes from that cursor with
/// its counters, never from the head, and walks on to the end.
#[tokio::test]
async fn a_re_asked_incomplete_walk_resumes_from_its_cursor_with_its_counters() {
    let (scraper, calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    hold_session(&resources, user_id, tenant, SIGNED_IN_SESSION).await;
    let ledger = &resources.common.repos.activity_backfill_jobs;
    assert!(
        ledger
            .record_backfill_job(&job_row(user_id, tenant, 0))
            .await
            .unwrap(),
        "bug: the ledger records the walk"
    );
    let cursor = (Timestamp::now() - Elapsed::days(6 * 365)).epoch_seconds();
    update_walk_row(
        &resources,
        user_id,
        "UPDATE activity_backfill_jobs SET outcome = 'incomplete', before_ts = $2, \
         months_done = 60, rows_landed = 7, ended_at_ms = $3, leased_until_ms = 0, \
         lease_token = NULL WHERE user_id = $1",
        &[
            cursor,
            Timestamp::now().epoch_millis() - PAST_THE_BACK_OFF_MS,
        ],
    )
    .await;

    assert!(
        ask_walk(&resources, user_id, tenant).await,
        "bug: an incomplete walk is asked again"
    );
    let row = walk_row(&resources, user_id, tenant).await;
    assert_eq!(
        row.state,
        BackfillJobState::Owed,
        "bug: the resumed walk is owed"
    );
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

    assert_eq!(
        walk_settled(&resources, user_id, tenant).await,
        BackfillJobState::Walked,
        "bug: the resumed walk reaches the end of the history"
    );
    let reads = session_reads(&calls);
    assert!(
        reads
            .first()
            .is_some_and(|read| query_bound(read, "before") == Some(cursor)),
        "bug: the resumed walk's first step reads below its cursor: {reads:?}"
    );
    assert!(
        reads
            .iter()
            .all(|read| query_bound(read, "before").is_some_and(|before| before <= cursor)),
        "bug: the resumed walk read above its cursor: {reads:?}"
    );
    assert_eq!(
        rows_under(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        1,
        "bug: only the run below the cursor is walked"
    );
    assert_eq!(
        walk_row(&resources, user_id, tenant)
            .await
            .progress
            .rows_landed,
        8,
        "bug: the run below the cursor is added to the counters kept"
    );
}
